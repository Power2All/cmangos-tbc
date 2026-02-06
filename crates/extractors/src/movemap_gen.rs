// movemap_gen.rs - MoveMap navigation mesh generator
// Port of contrib/mmap/src/ (MapBuilder, TerrainBuilder, IntermediateValues)
//
// Uses bundled Recast/Detour C++ source via cc crate + FFI wrapper.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
#[cfg(feature = "recast")]
use crate::recast_ffi;
use tracing::{debug, error, info, warn};

// ============================================================================
// Constants matching C++ (MapBuilder.h / TerrainBuilder.h / MoveMapSharedDefines.h)
// ============================================================================

/// World unit dimension for navmesh cells
const BASE_UNIT_DIM: f32 = 0.266_666_6;

/// Grid size in world units (one ADT tile)
const GRID_SIZE: f32 = 533.333_3;

/// Grid part size (one V8 cell)
const GRID_PART_SIZE: f32 = GRID_SIZE / V8_SIZE as f32;

/// Vertices per map tile (in recast cells)
const VERTEX_PER_MAP: i32 = (GRID_SIZE / BASE_UNIT_DIM + 0.5) as i32; // ~2000
/// Vertices per sub-tile
const VERTEX_PER_TILE: i32 = 80;
/// Sub-tiles per map tile
const TILES_PER_MAP: i32 = VERTEX_PER_MAP / VERTEX_PER_TILE; // 25

// Height grid sizes
const V9_SIZE: usize = 129;
const V9_SIZE_SQ: usize = V9_SIZE * V9_SIZE;
const V8_SIZE: usize = 128;
const V8_SIZE_SQ: usize = V8_SIZE * V8_SIZE;

// Liquid sentinel values
const INVALID_MAP_LIQ_HEIGHT: f32 = -500.0;
const INVALID_MAP_LIQ_HEIGHT_MAX: f32 = 5000.0;

// Map file version
const MAP_VERSION_MAGIC: &[u8; 4] = b"s1.4";

// Map file header flags
const MAP_HEIGHT_NO_HEIGHT: u32 = 0x0001;
const MAP_HEIGHT_AS_INT16: u32 = 0x0002;
const MAP_HEIGHT_AS_INT8: u32 = 0x0004;

const MAP_LIQUID_NO_TYPE: u8 = 0x01;
const MAP_LIQUID_NO_HEIGHT: u8 = 0x02;

// Liquid type flags
const MAP_LIQUID_TYPE_NO_WATER: u8 = 0x00;
const MAP_LIQUID_TYPE_MAGMA: u8 = 0x01;
const MAP_LIQUID_TYPE_OCEAN: u8 = 0x02;
const MAP_LIQUID_TYPE_SLIME: u8 = 0x04;
const MAP_LIQUID_TYPE_WATER: u8 = 0x08;
const MAP_LIQUID_TYPE_DEEP_WATER: u8 = 0x10;

// Nav area/terrain definitions
const NAV_AREA_EMPTY: u8 = 0;
const NAV_AREA_GROUND: u8 = 11;
const NAV_AREA_GROUND_STEEP: u8 = 10;
const NAV_AREA_WATER: u8 = 9;
const NAV_AREA_MAGMA_SLIME: u8 = 8;
const NAV_AREA_MAX_VALUE: u8 = NAV_AREA_GROUND;
const NAV_AREA_MIN_VALUE: u8 = NAV_AREA_MAGMA_SLIME;
const NAV_AREA_ALL_MASK: u8 = 0x3F;

const NAV_GROUND: u16 = 1 << (NAV_AREA_MAX_VALUE - NAV_AREA_GROUND);

// MMAP file format
const MMAP_MAGIC: u32 = 0x4d4d_4150; // 'MMAP'
const MMAP_VERSION: u32 = 8;

// Hole lookup tables
const HOLETAB_H: [u16; 4] = [0x1111, 0x2222, 0x4444, 0x8888];
const HOLETAB_V: [u16; 4] = [0x000F, 0x00F0, 0x0F00, 0xF000];

// ============================================================================
// Spot / Grid enums
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Spot {
    Top = 1,
    Right = 2,
    Left = 3,
    Bottom = 4,
    Entire = 5,
}

#[derive(Clone, Copy)]
enum Grid {
    V8,
    V9,
}

// ============================================================================
// MeshData - collects terrain/liquid/offmesh geometry
// ============================================================================

#[derive(Default)]
struct MeshData {
    solid_verts: Vec<f32>,
    solid_tris: Vec<i32>,
    liquid_verts: Vec<f32>,
    liquid_tris: Vec<i32>,
    liquid_type: Vec<u8>,

    // off-mesh connections
    off_mesh_connections: Vec<f32>,
    off_mesh_connection_rads: Vec<f32>,
    off_mesh_connection_dirs: Vec<u8>,
    off_mesh_connections_areas: Vec<u8>,
    off_mesh_connections_flags: Vec<u16>,
}

// ============================================================================
// VMap reader - reads .vmo model files and .vmtile/.vmtree references
// ============================================================================

/// Minimal vmap model instance data for MoveMapGen
#[derive(Clone, Debug)]
struct VmapModelInstance {
    name: String,
    pos: [f32; 3],
    rot: [f32; 3],
    scale: f32,
    flags: u32,
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    // liquid data from WMO groups
    liquid: Option<VmapLiquidData>,
}

#[derive(Clone, Debug)]
struct VmapLiquidData {
    tiles_x: u32,
    tiles_y: u32,
    corner: [f32; 3],
    liq_type: u32,
    heights: Vec<f32>,
    flags: Vec<u8>,
}

// ============================================================================
// TerrainBuilder
// ============================================================================

struct TerrainBuilder {
    skip_liquid: bool,
    maps_dir: PathBuf,
    vmaps_dir: PathBuf,
}

impl TerrainBuilder {
    fn new(skip_liquid: bool, maps_dir: &Path, vmaps_dir: &Path) -> Self {
        Self {
            skip_liquid,
            maps_dir: maps_dir.to_path_buf(),
            vmaps_dir: vmaps_dir.to_path_buf(),
        }
    }

    fn uses_liquids(&self) -> bool {
        !self.skip_liquid
    }

    /// Load terrain data for a tile and its adjacent borders
    fn load_map(&self, map_id: u32, tile_x: u32, tile_y: u32, mesh_data: &mut MeshData) {
        if self.load_map_portion(map_id, tile_x, tile_y, mesh_data, Spot::Entire) {
            self.load_map_portion(map_id, tile_x.wrapping_add(1), tile_y, mesh_data, Spot::Left);
            self.load_map_portion(map_id, tile_x.wrapping_sub(1), tile_y, mesh_data, Spot::Right);
            self.load_map_portion(map_id, tile_x, tile_y.wrapping_add(1), mesh_data, Spot::Top);
            self.load_map_portion(map_id, tile_x, tile_y.wrapping_sub(1), mesh_data, Spot::Bottom);
        }
    }

    /// Load a portion of terrain from a .map file
    fn load_map_portion(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh_data: &mut MeshData,
        portion: Spot,
    ) -> bool {
        let map_path = self.maps_dir.join(format!(
            "{:03}{:02}{:02}.map",
            map_id, tile_y, tile_x
        ));
        let mut file = match fs::File::open(&map_path) {
            Ok(f) => f,
            Err(_) => return false,
        };

        // Read file header
        let map_magic = read_u32_le(&mut file);
        let version_magic = read_u32_le(&mut file);
        let _area_map_offset = read_u32_le(&mut file);
        let _area_map_size = read_u32_le(&mut file);
        let height_map_offset = read_u32_le(&mut file);
        let _height_map_size = read_u32_le(&mut file);
        let liquid_map_offset = read_u32_le(&mut file);
        let _liquid_map_size = read_u32_le(&mut file);
        let holes_offset = read_u32_le(&mut file);
        let holes_size = read_u32_le(&mut file);

        // Verify version magic
        let expected_version = u32::from_le_bytes(*MAP_VERSION_MAGIC);
        if version_magic != expected_version {
            warn!(
                "{} is the wrong version, please extract new .map files",
                map_path.display()
            );
            return false;
        }

        // Read height header
        file.seek(SeekFrom::Start(height_map_offset as u64))
            .ok();
        let _hheader_fourcc = read_u32_le(&mut file);
        let hheader_flags = read_u32_le(&mut file);
        let grid_height = read_f32_le(&mut file);
        let grid_max_height = read_f32_le(&mut file);

        let have_terrain = (hheader_flags & MAP_HEIGHT_NO_HEIGHT) == 0;
        let have_liquid = liquid_map_offset != 0 && !self.skip_liquid;

        if !have_terrain && !have_liquid {
            return false;
        }

        // Data arrays
        let mut holes = [[0u16; 16]; 16];
        let mut liquid_entry = [[0u16; 16]; 16];
        let mut liquid_flags = [[0u8; 16]; 16];
        let mut liquid_type_loaded = false;
        let mut ltriangles: Vec<i32> = Vec::new();
        let mut ttriangles: Vec<i32> = Vec::new();

        // ---------- Terrain data ----------
        if have_terrain {
            let mut v9 = vec![0.0f32; V9_SIZE_SQ];
            let mut v8 = vec![0.0f32; V8_SIZE_SQ];

            if (hheader_flags & MAP_HEIGHT_AS_INT8) != 0 {
                let mut raw9 = vec![0u8; V9_SIZE_SQ];
                let mut raw8 = vec![0u8; V8_SIZE_SQ];
                file.read_exact(&mut raw9).ok();
                file.read_exact(&mut raw8).ok();
                let multiplier = (grid_max_height - grid_height) / 255.0;
                for i in 0..V9_SIZE_SQ {
                    v9[i] = raw9[i] as f32 * multiplier + grid_height;
                }
                for i in 0..V8_SIZE_SQ {
                    v8[i] = raw8[i] as f32 * multiplier + grid_height;
                }
            } else if (hheader_flags & MAP_HEIGHT_AS_INT16) != 0 {
                let mut raw9 = vec![0u16; V9_SIZE_SQ];
                let mut raw8 = vec![0u16; V8_SIZE_SQ];
                for v in raw9.iter_mut() {
                    *v = read_u16_le(&mut file);
                }
                for v in raw8.iter_mut() {
                    *v = read_u16_le(&mut file);
                }
                let multiplier = (grid_max_height - grid_height) / 65535.0;
                for i in 0..V9_SIZE_SQ {
                    v9[i] = raw9[i] as f32 * multiplier + grid_height;
                }
                for i in 0..V8_SIZE_SQ {
                    v8[i] = raw8[i] as f32 * multiplier + grid_height;
                }
            } else {
                for v in v9.iter_mut() {
                    *v = read_f32_le(&mut file);
                }
                for v in v8.iter_mut() {
                    *v = read_f32_le(&mut file);
                }
            }

            // Read hole data
            if holes_size > 0 {
                file.seek(SeekFrom::Start(holes_offset as u64)).ok();
                for row in holes.iter_mut() {
                    for col in row.iter_mut() {
                        *col = read_u16_le(&mut file);
                    }
                }
            }

            let count = (mesh_data.solid_verts.len() / 3) as i32;
            let xoffset = (tile_x as f32 - 32.0) * GRID_SIZE;
            let yoffset = (tile_y as f32 - 32.0) * GRID_SIZE;

            // V9 vertices
            for i in 0..V9_SIZE_SQ {
                let coord = get_height_coord(i, Grid::V9, xoffset, yoffset, &v9);
                mesh_data.solid_verts.push(coord[0]);
                mesh_data.solid_verts.push(coord[2]); // y,z swapped for recast
                mesh_data.solid_verts.push(coord[1]);
            }

            // V8 vertices
            for i in 0..V8_SIZE_SQ {
                let coord = get_height_coord(i, Grid::V8, xoffset, yoffset, &v8);
                mesh_data.solid_verts.push(coord[0]);
                mesh_data.solid_verts.push(coord[2]);
                mesh_data.solid_verts.push(coord[1]);
            }

            // Generate terrain triangles
            let (loop_start, loop_end, loop_inc) = get_loop_vars(portion);
            let mut i = loop_start;
            while i < loop_end {
                for j in [Spot::Top, Spot::Right, Spot::Left, Spot::Bottom] {
                    let indices = get_height_triangle(i, j, false);
                    ttriangles.push(indices[2] + count);
                    ttriangles.push(indices[1] + count);
                    ttriangles.push(indices[0] + count);
                }
                i += loop_inc;
            }
        }

        // ---------- Liquid data ----------
        if have_liquid {
            file.seek(SeekFrom::Start(liquid_map_offset as u64)).ok();

            // Liquid header
            let _liq_fourcc = read_u32_le(&mut file);
            let liq_flags = read_u8(&mut file);
            let liq_liquid_flags = read_u8(&mut file);
            let liq_liquid_type = read_u16_le(&mut file);
            let liq_offset_x = read_u8(&mut file);
            let liq_offset_y = read_u8(&mut file);
            let liq_width = read_u8(&mut file);
            let liq_height = read_u8(&mut file);
            let liq_liquid_level = read_f32_le(&mut file);

            if (liq_flags & MAP_LIQUID_NO_TYPE) == 0 {
                // Per-cell liquid entries and flags
                for row in liquid_entry.iter_mut() {
                    for col in row.iter_mut() {
                        *col = read_u16_le(&mut file);
                    }
                }
                for row in liquid_flags.iter_mut() {
                    for col in row.iter_mut() {
                        *col = read_u8(&mut file);
                    }
                }
                liquid_type_loaded = true;
            } else {
                // Use global values
                for row in liquid_entry.iter_mut() {
                    for col in row.iter_mut() {
                        *col = liq_liquid_type;
                    }
                }
                for row in liquid_flags.iter_mut() {
                    for col in row.iter_mut() {
                        *col = liq_liquid_flags;
                    }
                }
            }

            // Read liquid height map
            let mut liquid_map: Option<Vec<f32>> = None;
            if (liq_flags & MAP_LIQUID_NO_HEIGHT) == 0 {
                let data_size = liq_width as usize * liq_height as usize;
                let mut lmap = vec![0.0f32; data_size];
                for v in lmap.iter_mut() {
                    *v = read_f32_le(&mut file);
                }
                liquid_map = Some(lmap);
            }

            let count = (mesh_data.liquid_verts.len() / 3) as i32;
            let xoffset = (tile_x as f32 - 32.0) * GRID_SIZE;
            let yoffset = (tile_y as f32 - 32.0) * GRID_SIZE;

            // Generate liquid vertices
            if let Some(ref lmap) = liquid_map {
                let mut j = 0usize;
                for i in 0..V9_SIZE_SQ {
                    let row = i / V9_SIZE;
                    let col = i % V9_SIZE;

                    if row < liq_offset_y as usize
                        || row >= (liq_offset_y as usize + liq_height as usize)
                        || col < liq_offset_x as usize
                        || col >= (liq_offset_x as usize + liq_width as usize)
                    {
                        // dummy vert
                        mesh_data.liquid_verts.push(
                            -(xoffset + col as f32 * GRID_PART_SIZE),
                        );
                        mesh_data.liquid_verts.push(INVALID_MAP_LIQ_HEIGHT);
                        mesh_data.liquid_verts.push(
                            -(yoffset + row as f32 * GRID_PART_SIZE),
                        );
                        continue;
                    }

                    let coord = get_liquid_coord(i, j, xoffset, yoffset, lmap);
                    mesh_data.liquid_verts.push(coord[0]);
                    mesh_data.liquid_verts.push(coord[2]);
                    mesh_data.liquid_verts.push(coord[1]);
                    j += 1;
                }
            } else {
                for i in 0..V9_SIZE_SQ {
                    let row = i / V9_SIZE;
                    let col = i % V9_SIZE;
                    mesh_data.liquid_verts.push(
                        -(xoffset + col as f32 * GRID_PART_SIZE),
                    );
                    mesh_data.liquid_verts.push(liq_liquid_level);
                    mesh_data.liquid_verts.push(
                        -(yoffset + row as f32 * GRID_PART_SIZE),
                    );
                }
            }

            // Generate liquid triangles
            let (loop_start, loop_end, loop_inc) = get_loop_vars(portion);
            let tri_inc = Spot::Bottom as i32 - Spot::Top as i32; // 3
            let mut i = loop_start;
            while i < loop_end {
                for j_spot in [Spot::Top, Spot::Bottom] {
                    let indices = get_height_triangle(i, j_spot, true);
                    ltriangles.push(indices[2] + count);
                    ltriangles.push(indices[1] + count);
                    ltriangles.push(indices[0] + count);
                }
                i += loop_inc;
            }
        }

        // ---------- Resolve terrain vs liquid priority ----------
        if ltriangles.is_empty() && ttriangles.is_empty() {
            return false;
        }

        let t_tri_count = 4; // 4 terrain triangles per quad
        let lverts_copy: Vec<f32> = mesh_data.liquid_verts.clone();

        let (loop_start, loop_end, loop_inc) = get_loop_vars(portion);
        let mut lt_idx = 0usize; // liquid triangle index
        let mut tt_idx = 0usize; // terrain triangle index

        let mut i = loop_start;
        while i < loop_end {
            for _j in 0..2 {
                let mut use_terrain = true;
                let mut use_liquid = true;
                let mut liquid_type_val = MAP_LIQUID_TYPE_NO_WATER;

                // Check if liquid available
                if !liquid_type_loaded
                    || mesh_data.liquid_verts.is_empty()
                    || ltriangles.is_empty()
                {
                    use_liquid = false;
                } else {
                    liquid_type_val = get_liquid_type(i, &liquid_flags);
                    if (liquid_type_val & MAP_LIQUID_TYPE_DEEP_WATER) != 0 {
                        use_terrain = false;
                        use_liquid = false;
                    } else if (liquid_type_val
                        & (MAP_LIQUID_TYPE_WATER | MAP_LIQUID_TYPE_OCEAN))
                        != 0
                    {
                        liquid_type_val = NAV_AREA_WATER;
                    } else if (liquid_type_val & (MAP_LIQUID_TYPE_MAGMA | MAP_LIQUID_TYPE_SLIME)) != 0 {
                        liquid_type_val = NAV_AREA_MAGMA_SLIME;
                    } else {
                        use_liquid = false;
                    }
                }

                if ttriangles.is_empty() {
                    use_terrain = false;
                }

                // Patch missing liquid heights
                if use_liquid && lt_idx + 2 < ltriangles.len() {
                    let mut quad_height = 0.0f32;
                    let mut valid_count = 0u32;
                    for idx in 0..3 {
                        let vi = ltriangles[lt_idx + idx] as usize;
                        if vi * 3 + 1 < lverts_copy.len() {
                            let h = lverts_copy[vi * 3 + 1];
                            if h != INVALID_MAP_LIQ_HEIGHT && h < INVALID_MAP_LIQ_HEIGHT_MAX {
                                quad_height += h;
                                valid_count += 1;
                            }
                        }
                    }

                    if valid_count > 0 && valid_count < 3 {
                        quad_height /= valid_count as f32;
                        for idx in 0..3 {
                            let vi = ltriangles[lt_idx + idx] as usize;
                            if vi * 3 + 1 < mesh_data.liquid_verts.len() {
                                let h = mesh_data.liquid_verts[vi * 3 + 1];
                                if h == INVALID_MAP_LIQ_HEIGHT || h > INVALID_MAP_LIQ_HEIGHT_MAX {
                                    mesh_data.liquid_verts[vi * 3 + 1] = quad_height;
                                }
                            }
                        }
                    }

                    if valid_count == 0 {
                        use_liquid = false;
                    }
                }

                // Check terrain holes
                if use_terrain {
                    use_terrain = !is_hole(i, &holes);
                }

                // Choose higher surface
                if use_terrain && use_liquid {
                    let mut max_l_level = INVALID_MAP_LIQ_HEIGHT;
                    for x in 0..3 {
                        if lt_idx + x < ltriangles.len() {
                            let vi = ltriangles[lt_idx + x] as usize;
                            if vi * 3 + 1 < mesh_data.liquid_verts.len() {
                                let h = mesh_data.liquid_verts[vi * 3 + 1];
                                if max_l_level < h {
                                    max_l_level = h;
                                }
                            }
                        }
                    }

                    let mut min_t_level = INVALID_MAP_LIQ_HEIGHT_MAX;
                    let terrain_count = 3 * t_tri_count / 2; // 6
                    for x in 0..terrain_count {
                        if tt_idx + x < ttriangles.len() {
                            let vi = ttriangles[tt_idx + x] as usize;
                            if vi * 3 + 1 < mesh_data.solid_verts.len() {
                                let h = mesh_data.solid_verts[vi * 3 + 1];
                                if min_t_level > h {
                                    min_t_level = h;
                                }
                            }
                        }
                    }

                    if min_t_level > max_l_level {
                        use_liquid = false;
                    }
                }

                // Store results
                if use_liquid {
                    mesh_data.liquid_type.push(liquid_type_val);
                    for k in 0..3 {
                        if lt_idx + k < ltriangles.len() {
                            mesh_data.liquid_tris.push(ltriangles[lt_idx + k]);
                        }
                    }
                }

                if use_terrain {
                    let terrain_count = 3 * t_tri_count / 2;
                    for k in 0..terrain_count {
                        if tt_idx + k < ttriangles.len() {
                            mesh_data.solid_tris.push(ttriangles[tt_idx + k]);
                        }
                    }
                }

                lt_idx += 3;
                tt_idx += 3 * t_tri_count / 2;
            }
            i += loop_inc;
        }

        !mesh_data.solid_tris.is_empty() || !mesh_data.liquid_tris.is_empty()
    }

    /// Load VMap model data for a tile
    fn load_vmap(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh_data: &mut MeshData,
    ) -> bool {
        // Load the vmtree file to find model instances
        let vmtree_path = self.vmaps_dir.join(format!("{:03}.vmtree", map_id));
        let vmtree_data = match fs::read(&vmtree_path) {
            Ok(d) => d,
            Err(_) => return false,
        };

        if vmtree_data.len() < 12 {
            return false;
        }

        // Parse vmtree to find model instances for this tile
        // The vmtree format: magic(8) + isTiled(u32) + ...
        let mut cursor = std::io::Cursor::new(&vmtree_data);
        let mut magic_buf = [0u8; 8];
        if cursor.read_exact(&mut magic_buf).is_err() {
            return false;
        }

        let is_tiled = read_u32_le(&mut cursor);

        // Read BIH tree (skip over it) - bounds(6 floats) + tree_size(u32) + tree[tree_size] + obj_count(u32) + objs[obj_count]
        let bmin_x = read_f32_le(&mut cursor);
        let bmin_y = read_f32_le(&mut cursor);
        let bmin_z = read_f32_le(&mut cursor);
        let bmax_x = read_f32_le(&mut cursor);
        let bmax_y = read_f32_le(&mut cursor);
        let bmax_z = read_f32_le(&mut cursor);

        let tree_size = read_u32_le(&mut cursor);
        // Skip tree data
        let pos = cursor.position() + tree_size as u64 * 4;
        cursor.set_position(pos);

        let obj_count = read_u32_le(&mut cursor);
        // Skip obj data
        let pos = cursor.position() + obj_count as u64 * 4;
        cursor.set_position(pos);

        // Read model spawn count and spawns
        let n_values = read_u32_le(&mut cursor);

        let mut retval = false;

        for _i in 0..n_values {
            // Read ModelSpawn
            let spawn = match read_model_spawn(&mut cursor) {
                Some(s) => s,
                None => break,
            };

            // Load the actual model
            let model_path = self.vmaps_dir.join(&spawn.name);
            let world_model = match load_world_model(&model_path) {
                Some(m) => m,
                None => continue,
            };

            retval = true;

            let is_m2 = spawn.name.contains(".m2") || spawn.name.contains(".M2");

            // Transform data
            let scale = spawn.scale;
            let rotation = matrix3_from_euler_xyz(
                std::f32::consts::PI * spawn.rot[2] / -180.0,
                std::f32::consts::PI * spawn.rot[0] / -180.0,
                std::f32::consts::PI * spawn.rot[1] / -180.0,
            );
            let mut position = spawn.pos;
            position[0] -= 32.0 * GRID_SIZE;
            position[1] -= 32.0 * GRID_SIZE;

            for group in &world_model.groups {
                // Transform and add solid mesh
                let transformed: Vec<[f32; 3]> = group
                    .vertices
                    .iter()
                    .map(|v| {
                        let rotated = mat3_mul_vec3(&rotation, v);
                        let mut result = [
                            rotated[0] * scale + position[0],
                            rotated[1] * scale + position[1],
                            rotated[2] * scale + position[2],
                        ];
                        result[0] *= -1.0;
                        result[1] *= -1.0;
                        result
                    })
                    .collect();

                let offset = (mesh_data.solid_verts.len() / 3) as i32;

                // Copy vertices (y, z, x order for Recast)
                for v in &transformed {
                    mesh_data.solid_verts.push(v[1]);
                    mesh_data.solid_verts.push(v[2]);
                    mesh_data.solid_verts.push(v[0]);
                }

                // Copy indices (flipped for M2)
                for tri in &group.triangles {
                    if is_m2 {
                        mesh_data.solid_tris.push(tri[2] as i32 + offset);
                        mesh_data.solid_tris.push(tri[1] as i32 + offset);
                        mesh_data.solid_tris.push(tri[0] as i32 + offset);
                    } else {
                        mesh_data.solid_tris.push(tri[0] as i32 + offset);
                        mesh_data.solid_tris.push(tri[1] as i32 + offset);
                        mesh_data.solid_tris.push(tri[2] as i32 + offset);
                    }
                }

                // Handle liquid data
                if let Some(ref liquid) = group.liquid {
                    let verts_x = liquid.tiles_x + 1;
                    let verts_y = liquid.tiles_y + 1;

                    let liq_type = match liquid.liq_type & 3 {
                        0 | 1 => NAV_AREA_WATER,
                        2 | 3 => NAV_AREA_MAGMA_SLIME,
                        _ => NAV_AREA_WATER,
                    };

                    let mut liq_verts: Vec<[f32; 3]> = Vec::new();
                    let mut liq_tris: Vec<[i32; 3]> = Vec::new();

                    for x in 0..verts_x {
                        for y in 0..verts_y {
                            let vert = [
                                liquid.corner[0] + x as f32 * GRID_PART_SIZE,
                                liquid.corner[1] + y as f32 * GRID_PART_SIZE,
                                liquid.heights[(y * verts_x + x) as usize],
                            ];
                            // transform: v * rotation * scale + position, then mirror x,y
                            let rotated = mat3_mul_vec3(&rotation, &vert);
                            let mut result = [
                                rotated[0] * scale + position[0],
                                rotated[1] * scale + position[1],
                                rotated[2] * scale + position[2],
                            ];
                            result[0] *= -1.0;
                            result[1] *= -1.0;
                            liq_verts.push(result);
                        }
                    }

                    for x in 0..liquid.tiles_x {
                        for y in 0..liquid.tiles_y {
                            let flag_idx = (x + y * liquid.tiles_x) as usize;
                            if flag_idx < liquid.flags.len()
                                && (liquid.flags[flag_idx] & 0x0f) != 0x0f
                            {
                                let square = x * liquid.tiles_y + y;
                                let idx1 = (square + x) as i32;
                                let idx2 = (square + 1 + x) as i32;
                                let idx3 = (square + liquid.tiles_y + 1 + 1 + x) as i32;
                                let idx4 = (square + liquid.tiles_y + 1 + x) as i32;

                                liq_tris.push([idx3, idx2, idx1]);
                                liq_tris.push([idx4, idx3, idx1]);
                            }
                        }
                    }

                    let liq_offset = (mesh_data.liquid_verts.len() / 3) as i32;
                    for v in &liq_verts {
                        mesh_data.liquid_verts.push(v[1]); // y
                        mesh_data.liquid_verts.push(v[2]); // z
                        mesh_data.liquid_verts.push(v[0]); // x
                    }

                    for tri in &liq_tris {
                        mesh_data
                            .liquid_tris
                            .push(tri[1] + liq_offset);
                        mesh_data
                            .liquid_tris
                            .push(tri[2] + liq_offset);
                        mesh_data
                            .liquid_tris
                            .push(tri[0] + liq_offset);
                        mesh_data.liquid_type.push(liq_type);
                    }
                }
            }
        }

        retval
    }

    /// Load off-mesh connections from file
    fn load_off_mesh_connections(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh_data: &mut MeshData,
        off_mesh_file_path: Option<&Path>,
    ) {
        let path = match off_mesh_file_path {
            Some(p) => p,
            None => return,
        };

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(_) => {
                debug!("loadOffMeshConnections: input file {:?} not found", path);
                return;
            }
        };

        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Format: mapID tileX,tileY (p0x p0y p0z) (p1x p1y p1z) size
            // We need to parse this carefully
            if parts.len() < 10 {
                continue;
            }
            let mid: u32 = match parts[0].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let tile_parts: Vec<&str> = parts[1].split(',').collect();
            if tile_parts.len() != 2 {
                continue;
            }
            let tx: u32 = match tile_parts[0].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ty: u32 = match tile_parts[1].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Remove parentheses and parse coordinates
            let clean_line = line
                .replace(['(', ')'], "");
            let clean_parts: Vec<&str> = clean_line.split_whitespace().collect();
            if clean_parts.len() < 10 {
                continue;
            }

            let p0x: f32 = clean_parts[2].parse().unwrap_or(0.0);
            let p0y: f32 = clean_parts[3].parse().unwrap_or(0.0);
            let p0z: f32 = clean_parts[4].parse().unwrap_or(0.0);
            let p1x: f32 = clean_parts[5].parse().unwrap_or(0.0);
            let p1y: f32 = clean_parts[6].parse().unwrap_or(0.0);
            let p1z: f32 = clean_parts[7].parse().unwrap_or(0.0);
            let size: f32 = clean_parts[8].parse().unwrap_or(0.0);

            if mid == map_id && tx == tile_x && ty == tile_y {
                // Swap coordinates (y, z, x) for recast
                mesh_data.off_mesh_connections.push(p0y);
                mesh_data.off_mesh_connections.push(p0z);
                mesh_data.off_mesh_connections.push(p0x);
                mesh_data.off_mesh_connections.push(p1y);
                mesh_data.off_mesh_connections.push(p1z);
                mesh_data.off_mesh_connections.push(p1x);

                mesh_data.off_mesh_connection_dirs.push(1); // bidirectional
                mesh_data.off_mesh_connection_rads.push(size);
                mesh_data.off_mesh_connections_areas.push(0xFF);
                mesh_data.off_mesh_connections_flags.push(0xFF);
            }
        }
    }
}

// ============================================================================
// MapBuilder configuration (JSON config)
// ============================================================================

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MmapConfig {
    #[serde(default = "default_border_size")]
    border_size: i32,
    #[serde(default = "default_walkable_slope_angle")]
    walkable_slope_angle: f32,
    #[serde(default = "default_walkable_height")]
    walkable_height: i32,
    #[serde(default = "default_walkable_climb")]
    walkable_climb: i32,
    #[serde(default = "default_walkable_radius")]
    walkable_radius: i32,
    #[serde(default = "default_max_edge_len")]
    max_edge_len: i32,
    #[serde(default = "default_max_simplification_error")]
    max_simplification_error: f32,
    #[serde(default = "default_min_region_area")]
    min_region_area: i32,
    #[serde(default = "default_merge_region_area")]
    merge_region_area: i32,
    #[serde(default = "default_detail_sample_dist")]
    detail_sample_dist: f32,
    #[serde(default = "default_detail_sample_max_error")]
    detail_sample_max_error: f32,
    #[serde(default)]
    liquid_flag_merge_threshold: f32,
}

fn default_border_size() -> i32 { 5 }
fn default_walkable_slope_angle() -> f32 { 60.0 }
fn default_walkable_height() -> i32 { 6 }
fn default_walkable_climb() -> i32 { 4 }
fn default_walkable_radius() -> i32 { 2 }
fn default_max_edge_len() -> i32 { VERTEX_PER_TILE + 1 }
fn default_max_simplification_error() -> f32 { 1.8 }
fn default_min_region_area() -> i32 { 60 }
fn default_merge_region_area() -> i32 { 50 }
fn default_detail_sample_dist() -> f32 { BASE_UNIT_DIM * 16.0 }
fn default_detail_sample_max_error() -> f32 { BASE_UNIT_DIM }

impl Default for MmapConfig {
    fn default() -> Self {
        Self {
            border_size: default_border_size(),
            walkable_slope_angle: default_walkable_slope_angle(),
            walkable_height: default_walkable_height(),
            walkable_climb: default_walkable_climb(),
            walkable_radius: default_walkable_radius(),
            max_edge_len: default_max_edge_len(),
            max_simplification_error: default_max_simplification_error(),
            min_region_area: default_min_region_area(),
            merge_region_area: default_merge_region_area(),
            detail_sample_dist: default_detail_sample_dist(),
            detail_sample_max_error: default_detail_sample_max_error(),
            liquid_flag_merge_threshold: 0.0,
        }
    }
}

impl MmapConfig {
    fn to_rc_config(&self) -> RcConfig {
        RcConfig {
            tile_size: VERTEX_PER_TILE,
            border_size: self.border_size,
            cs: BASE_UNIT_DIM,
            ch: BASE_UNIT_DIM,
            walkable_slope_angle: self.walkable_slope_angle,
            walkable_height: self.walkable_height,
            walkable_climb: self.walkable_climb,
            walkable_radius: self.walkable_radius,
            max_edge_len: self.max_edge_len,
            max_simplification_error: self.max_simplification_error,
            min_region_area: self.min_region_area * self.min_region_area, // rcSqr
            merge_region_area: self.merge_region_area * self.merge_region_area, // rcSqr
            max_verts_per_poly: DT_VERTS_PER_POLYGON as i32,
            detail_sample_dist: self.detail_sample_dist,
            detail_sample_max_error: self.detail_sample_max_error,
            ..Default::default()
        }
    }
}

/// Recast config (mirrors rcConfig struct)
#[derive(Clone, Default)]
struct RcConfig {
    width: i32,
    height: i32,
    tile_size: i32,
    border_size: i32,
    cs: f32,
    ch: f32,
    bmin: [f32; 3],
    bmax: [f32; 3],
    walkable_slope_angle: f32,
    walkable_height: i32,
    walkable_climb: i32,
    walkable_radius: i32,
    max_edge_len: i32,
    max_simplification_error: f32,
    min_region_area: i32,
    merge_region_area: i32,
    max_verts_per_poly: i32,
    detail_sample_dist: f32,
    detail_sample_max_error: f32,
}

/// DT_VERTS_PER_POLYGON from Detour
const DT_VERTS_PER_POLYGON: u32 = 6;

/// DT_NAVMESH_VERSION - matches the version from the bundled Detour
const DT_NAVMESH_VERSION_CONST: u32 = 7;

/// DT_POLY_BITS
const DT_POLY_BITS: u32 = 20;

// ============================================================================
// MapBuilder
// ============================================================================

struct MapBuilder {
    terrain_builder: TerrainBuilder,
    tiles: BTreeMap<u32, BTreeSet<u32>>,
    debug: bool,
    off_mesh_file_path: Option<PathBuf>,
    maps_dir: PathBuf,
    vmaps_dir: PathBuf,
    mmaps_dir: PathBuf,
    skip_continents: bool,
    skip_junk_maps: bool,
    skip_battlegrounds: bool,
    config: Option<serde_json::Value>,
    map_done: BTreeSet<u32>,
    threads: usize,
}

impl MapBuilder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config_input_path: Option<&Path>,
        threads: usize,
        skip_liquid: bool,
        skip_continents: bool,
        skip_junk_maps: bool,
        skip_battlegrounds: bool,
        debug: bool,
        off_mesh_file_path: Option<&Path>,
        maps_dir: &Path,
        vmaps_dir: &Path,
        mmaps_dir: &Path,
    ) -> Self {
        let config = config_input_path.and_then(|p| {
            fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
        });

        let terrain_builder = TerrainBuilder::new(skip_liquid, maps_dir, vmaps_dir);

        info!("Using {} thread(s) for processing.", threads);

        let mut builder = Self {
            terrain_builder,
            tiles: BTreeMap::new(),
            debug,
            off_mesh_file_path: off_mesh_file_path.map(|p| p.to_path_buf()),
            maps_dir: maps_dir.to_path_buf(),
            vmaps_dir: vmaps_dir.to_path_buf(),
            mmaps_dir: mmaps_dir.to_path_buf(),
            skip_continents,
            skip_junk_maps,
            skip_battlegrounds,
            config,
            map_done: BTreeSet::new(),
            threads,
        };

        builder.discover_tiles();
        builder
    }

    /// Scan maps/ and vmaps/ directories for available tiles
    fn discover_tiles(&mut self) {
        let maps_dir = &self.maps_dir;
        let vmaps_dir = &self.vmaps_dir;

        info!("Discovering maps...");
        let mut count = 0u32;

        // Scan maps/ directory
        if let Ok(entries) = fs::read_dir(maps_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.len() >= 3
                    && let Ok(map_id) = name[..3].parse::<u32>()
                    && let std::collections::btree_map::Entry::Vacant(e) = self.tiles.entry(map_id)
                {
                    e.insert(BTreeSet::new());
                    count += 1;
                }
            }
        }

        // Scan vmaps/ for .vmtree files
        if let Ok(entries) = fs::read_dir(vmaps_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".vmtree") && name.len() >= 3
                    && let Ok(map_id) = name[..3].parse::<u32>()
                    && let std::collections::btree_map::Entry::Vacant(e) = self.tiles.entry(map_id)
                {
                    e.insert(BTreeSet::new());
                    count += 1;
                }
            }
        }
        info!("found {} maps.", count);

        // Discover tiles per map
        count = 0;
        info!("Discovering tiles...");
        let map_ids: Vec<u32> = self.tiles.keys().cloned().collect();
        for map_id in map_ids {
            // Scan vmaps for .vmtile files
            if let Ok(entries) = fs::read_dir(vmaps_dir) {
                let filter = format!("{:03}", map_id);
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&filter) && name.ends_with(".vmtile") && name.len() >= 9 {
                        // Format: MMMYYtXX.vmtile
                        if let (Ok(tile_y), Ok(tile_x)) = (
                            name[3..5].parse::<u32>(),
                            name[6..8].parse::<u32>(),
                        ) {
                            let tile_id = pack_tile_id(tile_y, tile_x);
                            if self.tiles.get_mut(&map_id).unwrap().insert(tile_id) {
                                count += 1;
                            }
                        }
                    }
                }
            }

            // Scan maps for .map files
            if let Ok(entries) = fs::read_dir(maps_dir) {
                let filter = format!("{:03}", map_id);
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&filter) && name.ends_with(".map") && name.len() >= 7 {
                        // Format: MMMYYXX.map
                        if let (Ok(tile_y), Ok(tile_x)) = (
                            name[3..5].parse::<u32>(),
                            name[5..7].parse::<u32>(),
                        ) {
                            let tile_id = pack_tile_id(tile_x, tile_y);
                            if self.tiles.get_mut(&map_id).unwrap().insert(tile_id) {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
        info!("found {} tiles.\n", count);
    }

    /// Build navigation meshes for all (or specified) maps
    fn build_maps(&mut self, ids: &[u32]) {
        if ids.is_empty() {
            let map_ids: Vec<u32> = self.tiles.keys().cloned().collect();
            for map_id in map_ids {
                if !self.should_skip_map(map_id) {
                    self.build_map(map_id);
                }
                self.map_done.insert(map_id);
            }
        } else {
            for &map_id in ids {
                if !self.should_skip_map(map_id) {
                    self.build_map(map_id);
                }
                self.map_done.insert(map_id);
            }
        }
    }

    /// Build a single tile
    fn build_single_tile(&mut self, map_id: u32, tile_x: u32, tile_y: u32) {
        let nav_mesh_params = match self.build_nav_mesh(map_id) {
            Some(p) => p,
            None => {
                error!("[Map {:03}] Failed creating navmesh!", map_id);
                return;
            }
        };

        self.build_tile(map_id, tile_x, tile_y, &nav_mesh_params, 1, 1);
    }

    /// Build all tiles for a map
    fn build_map(&mut self, map_id: u32) {
        info!("Building map {:03}:", map_id);

        let tiles: Vec<u32> = self.tiles.get(&map_id).cloned().unwrap_or_default().into_iter().collect();

        if tiles.is_empty() {
            return;
        }

        let nav_mesh_params = match self.build_nav_mesh(map_id) {
            Some(p) => p,
            None => {
                error!("[Map {:03}] Failed creating navmesh!", map_id);
                return;
            }
        };

        info!("[Map {:03}] We have {} tiles.", map_id, tiles.len());

        // Build tiles using thread pool
        let tile_count = tiles.len() as u32;
        let mmaps_dir = self.mmaps_dir.clone();
        let terrain_builder = Arc::new(TerrainBuilder::new(
            self.terrain_builder.skip_liquid,
            &self.maps_dir,
            &self.vmaps_dir,
        ));
        let off_mesh_path = self.off_mesh_file_path.clone();
        let debug = self.debug;
        let config_json = self.config.clone();

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.threads)
            .build();

        match pool {
            Ok(pool) => {
                pool.scope(|s| {
                    for (idx, &tile_packed) in tiles.iter().enumerate() {
                        let (tile_x, tile_y) = unpack_tile_id(tile_packed);
                        let cur_tile = (idx + 1) as u32;
                        let nav_params = nav_mesh_params.clone();
                        let mmaps_dir = mmaps_dir.clone();
                        let tb = terrain_builder.clone();
                        let omp = off_mesh_path.clone();
                        let cfg_json = config_json.clone();

                        s.spawn(move |_| {
                            build_tile_worker(
                                map_id, tile_x, tile_y, &nav_params, cur_tile, tile_count,
                                &mmaps_dir, &tb, omp.as_deref(), debug, &cfg_json,
                            );
                        });
                    }
                });
            }
            Err(e) => {
                // Fallback to single-threaded
                warn!("Failed to create thread pool: {}, using single-threaded", e);
                for (idx, &tile_packed) in tiles.iter().enumerate() {
                    let (tile_x, tile_y) = unpack_tile_id(tile_packed);
                    build_tile_worker(
                        map_id, tile_x, tile_y, &nav_mesh_params,
                        (idx + 1) as u32, tile_count,
                        &mmaps_dir, &terrain_builder, off_mesh_path.as_deref(),
                        debug, &config_json,
                    );
                }
            }
        }
    }

    /// Build tile (single-threaded path used for build_single_tile)
    fn build_tile(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        nav_mesh_params: &NavMeshParams,
        cur_tile: u32,
        tile_count: u32,
    ) {
        let tb = TerrainBuilder::new(
            self.terrain_builder.skip_liquid,
            &self.maps_dir,
            &self.vmaps_dir,
        );
        build_tile_worker(
            map_id, tile_x, tile_y, nav_mesh_params, cur_tile, tile_count,
            &self.mmaps_dir, &tb, self.off_mesh_file_path.as_deref(),
            self.debug, &self.config,
        );
    }

    /// Create and write the navmesh parameters (.mmap file)
    fn build_nav_mesh(&self, map_id: u32) -> Option<NavMeshParams> {
        let tiles = self.tiles.get(&map_id)?;
        if tiles.is_empty() {
            return None;
        }

        let poly_bits = DT_POLY_BITS;
        let max_tiles = tiles.len() as i32;
        let max_polys_per_tile = 1i32 << poly_bits;

        // Calculate bounds
        let mut tile_x_max = 0u32;
        let mut tile_y_max = 0u32;
        for &tile_packed in tiles {
            let (tx, ty) = unpack_tile_id(tile_packed);
            if tx > tile_x_max { tile_x_max = tx; }
            if ty > tile_y_max { tile_y_max = ty; }
        }

        let (bmin, bmax) = get_tile_bounds(tile_x_max, tile_y_max, &[], 0);

        let params = NavMeshParams {
            orig: bmin,
            tile_width: GRID_SIZE,
            tile_height: GRID_SIZE,
            max_tiles,
            max_polys: max_polys_per_tile,
        };

        // Write .mmap file
        let file_name = self.mmaps_dir.join(format!("{:03}.mmap", map_id));
        match write_nav_mesh_params(&file_name, &params) {
            Ok(_) => info!("[Map {:03}] Created navMesh params", map_id),
            Err(e) => {
                error!("[Map {:03}] Failed to write {}: {}", map_id, file_name.display(), e);
                return None;
            }
        }

        Some(params)
    }

    fn should_skip_map(&self, map_id: u32) -> bool {
        if self.skip_continents {
            match map_id {
                0 | 1 | 530 => return true,
                _ => {}
            }
        }

        if self.skip_junk_maps {
            match map_id {
                13 | 25 | 29 | 42 | 169 | 451 => return true,
                _ => {
                    if is_transport_map(map_id) {
                        return true;
                    }
                }
            }
        }

        if self.skip_battlegrounds {
            match map_id {
                30 | 37 | 489 | 529 | 566 => return true,
                _ => {}
            }
        }

        false
    }

    fn should_skip_tile(&self, map_id: u32, tile_x: u32, tile_y: u32) -> bool {
        let file_name = self.mmaps_dir.join(format!(
            "{:03}{:02}{:02}.mmtile",
            map_id, tile_y, tile_x
        ));
        let mut file = match fs::File::open(&file_name) {
            Ok(f) => f,
            Err(_) => return false,
        };

        // Read header
        let mmap_magic = read_u32_le(&mut file);
        let dt_version = read_u32_le(&mut file);
        let mmap_version = read_u32_le(&mut file);
        let _size = read_u32_le(&mut file);
        let _uses_liquids = read_u32_le(&mut file);

        if mmap_magic != MMAP_MAGIC || dt_version != DT_NAVMESH_VERSION_CONST {
            return false;
        }
        if mmap_version != MMAP_VERSION {
            return false;
        }

        true
    }

    /// Build transports
    fn build_transports(&mut self) {
        self.build_game_object("Transportship.wmo.vmo", 3015);
        self.build_game_object("Transport_Zeppelin.wmo.vmo", 3031);
        self.build_game_object("Transportship_Ne.wmo.vmo", 7087);
        self.build_game_object("Elevatorcar.m2.vmo", 360);
        self.build_game_object("Undeadelevator.m2.vmo", 455);
        self.build_game_object("Ironforgeelevator.m2.vmo", 561);
        self.build_game_object("Gnomeelevatorcar01.m2.vmo", 807);
        self.build_game_object("Gnomeelevatorcar02.m2.vmo", 808);
        self.build_game_object("Gnomeelevatorcar03.m2.vmo", 827);
        self.build_game_object("Gnomeelevatorcar03.m2.vmo", 852);
        self.build_game_object("Gnomehutelevator.m2.vmo", 1587);
        self.build_game_object("Burningsteppselevator.m2.vmo", 2454);
        self.build_game_object("Subwaycar.m2.vmo", 3831);
        // TBC+
        self.build_game_object("Ancdrae_Elevatorpiece.m2.vmo", 7026);
        self.build_game_object("Mushroombase_Elevator.m2.vmo", 7028);
        self.build_game_object("Cf_Elevatorplatform.m2.vmo", 7043);
        self.build_game_object("Cf_Elevatorplatform_Small.m2.vmo", 7060);
        self.build_game_object("Factoryelevator.m2.vmo", 7077);
        self.build_game_object("Ancdrae_Elevatorpiece_Netherstorm.m2.vmo", 7163);
    }

    /// Build navmesh for a game object (transport/elevator)
    fn build_game_object(&self, model_name: &str, display_id: u32) {
        let full_path = self.vmaps_dir.join(model_name);

        info!("Building GameObject model {}", model_name);

        let world_model = match load_world_model(&full_path) {
            Some(m) => m,
            None => {
                warn!("* Unable to open file {:?}", full_path);
                return;
            }
        };

        let mut mesh_data = MeshData::default();
        let is_m2 = model_name.contains(".m2") || model_name.contains(".M2");

        for group in &world_model.groups {
            let offset = (mesh_data.solid_verts.len() / 3) as i32;

            for v in &group.vertices {
                mesh_data.solid_verts.push(v[1]); // y
                mesh_data.solid_verts.push(v[2]); // z
                mesh_data.solid_verts.push(v[0]); // x
            }

            for tri in &group.triangles {
                if is_m2 {
                    mesh_data.solid_tris.push(tri[2] as i32 + offset);
                    mesh_data.solid_tris.push(tri[1] as i32 + offset);
                    mesh_data.solid_tris.push(tri[0] as i32 + offset);
                } else {
                    mesh_data.solid_tris.push(tri[0] as i32 + offset);
                    mesh_data.solid_tris.push(tri[1] as i32 + offset);
                    mesh_data.solid_tris.push(tri[2] as i32 + offset);
                }
            }
        }

        if mesh_data.solid_verts.is_empty() {
            warn!("* no solid vertices found");
            return;
        }

        clean_vertices(&mut mesh_data.solid_verts, &mut mesh_data.solid_tris);
        info!("* Model opened ({} vertices)", mesh_data.solid_verts.len());

        // Build using recast (simplified - no tiling for GO)
        // TODO: implement GO navmesh building using recast FFI
        // For now, write a placeholder note
        info!(
            "* GO navmesh building for display {} - requires Recast FFI (TODO)",
            display_id
        );
    }
}

// ============================================================================
// NavMeshParams - serializable navmesh parameters
// ============================================================================

#[derive(Clone, Debug)]
struct NavMeshParams {
    orig: [f32; 3],
    tile_width: f32,
    tile_height: f32,
    max_tiles: i32,
    max_polys: i32,
}

fn write_nav_mesh_params(path: &Path, params: &NavMeshParams) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    // dtNavMeshParams layout: orig[3], tileWidth, tileHeight, maxTiles, maxPolys
    for v in &params.orig {
        file.write_f32::<LittleEndian>(*v)?;
    }
    file.write_f32::<LittleEndian>(params.tile_width)?;
    file.write_f32::<LittleEndian>(params.tile_height)?;
    file.write_i32::<LittleEndian>(params.max_tiles)?;
    file.write_i32::<LittleEndian>(params.max_polys)?;
    Ok(())
}

// ============================================================================
// Tile building worker (called per-tile, potentially from thread pool)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn build_tile_worker(
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
    nav_mesh_params: &NavMeshParams,
    cur_tile: u32,
    tile_count: u32,
    mmaps_dir: &Path,
    terrain_builder: &TerrainBuilder,
    off_mesh_file_path: Option<&Path>,
    debug: bool,
    config_json: &Option<serde_json::Value>,
) {
    info!(
        "[Map {:03}] Building tile [{:02},{:02}] ({:02} / {:02})",
        map_id, tile_x, tile_y, cur_tile, tile_count
    );

    let mut mesh_data = MeshData::default();

    // Load heightmap data
    terrain_builder.load_map(map_id, tile_x, tile_y, &mut mesh_data);

    // Load vmap data (note: C++ passes tileY,tileX for vmap but tileX,tileY for map)
    terrain_builder.load_vmap(map_id, tile_y, tile_x, &mut mesh_data);

    if mesh_data.solid_verts.is_empty() && mesh_data.liquid_verts.is_empty() {
        return;
    }

    // Clean unused vertices
    clean_vertices(&mut mesh_data.solid_verts, &mut mesh_data.solid_tris);
    clean_vertices(&mut mesh_data.liquid_verts, &mut mesh_data.liquid_tris);

    // Gather all verts for bounds calculation
    let mut all_verts: Vec<f32> = Vec::new();
    all_verts.extend_from_slice(&mesh_data.liquid_verts);
    all_verts.extend_from_slice(&mesh_data.solid_verts);

    if all_verts.is_empty() {
        return;
    }

    // Get tile bounds
    let (bmin, bmax) = get_tile_bounds(tile_x, tile_y, &all_verts, all_verts.len() / 3);

    // Load off-mesh connections
    terrain_builder.load_off_mesh_connections(
        map_id,
        tile_x,
        tile_y,
        &mut mesh_data,
        off_mesh_file_path,
    );

    // Build the move map tile
    build_move_map_tile(
        map_id, tile_x, tile_y, &mut mesh_data, &bmin, &bmax, nav_mesh_params,
        mmaps_dir, terrain_builder.uses_liquids(), debug, config_json,
    );
}

/// Build the actual navmesh tile using Recast pipeline
#[allow(clippy::too_many_arguments)]
fn build_move_map_tile(
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
    mesh_data: &mut MeshData,
    bmin: &[f32; 3],
    bmax: &[f32; 3],
    nav_mesh_params: &NavMeshParams,
    mmaps_dir: &Path,
    uses_liquids: bool,
    debug: bool,
    config_json: &Option<serde_json::Value>,
) {
    let tile_string = format!("[Map {:03}] [{:02},{:02}]", map_id, tile_x, tile_y);
    info!("{}: Building movemap tiles...", tile_string);

    let t_verts = &mesh_data.solid_verts;
    let t_vert_count = t_verts.len() / 3;
    let t_tris = &mesh_data.solid_tris;
    let t_tri_count = t_tris.len() / 3;

    let l_verts = &mesh_data.liquid_verts;
    let l_vert_count = l_verts.len() / 3;
    let l_tris = &mesh_data.liquid_tris;
    let l_tri_count = l_tris.len() / 3;
    let l_tri_flags = &mesh_data.liquid_type;

    // Get configuration for this tile
    let mmap_config = get_tile_config(config_json, map_id, tile_x, tile_y);
    let mut config = mmap_config.to_rc_config();
    config.bmin = *bmin;
    config.bmax = *bmax;

    // Calculate grid size (used by the Recast pipeline when feature enabled)
    #[allow(unused_assignments)]
    {
        config.width =
            ((config.bmax[0] - config.bmin[0]) / config.cs + 0.5) as i32;
        config.height =
            ((config.bmax[2] - config.bmin[2]) / config.cs + 0.5) as i32;
    }

    // Build sub-tiles using Recast pipeline
    // This is where we'd call the actual Recast FFI functions.
    // For now, we build the navmesh data using safe abstractions over the FFI.

    #[cfg(feature = "recast")]
    unsafe {
        build_move_map_tile_unsafe(
            &tile_string,
            map_id, tile_x, tile_y,
            mesh_data,
            bmin, bmax,
            nav_mesh_params,
            &config,
            mmaps_dir,
            uses_liquids,
        );
    }

    #[cfg(not(feature = "recast"))]
    {
        warn!(
            "{}: Recast FFI not available (build with --features recast). \
             Terrain data loaded ({} solid verts, {} liquid verts) but navmesh not built.",
            tile_string,
            mesh_data.solid_verts.len() / 3,
            mesh_data.liquid_verts.len() / 3,
        );
    }
}

#[cfg(feature = "recast")]
/// Core Recast/Detour tile building - requires unsafe for FFI
#[allow(clippy::too_many_arguments)]
unsafe fn build_move_map_tile_unsafe(
    tile_string: &str,
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
    mesh_data: &MeshData,
    bmin: &[f32; 3],
    bmax: &[f32; 3],
    nav_mesh_params: &NavMeshParams,
    config: &RcConfig,
    mmaps_dir: &Path,
    uses_liquids: bool,
) {
    use recast_ffi::*;
    use std::io::Write;
    unsafe {

    // Create Recast context
    let ctx = rc_alloc_context();
    if ctx.is_null() {
        error!("{} Failed to allocate recast context!", tile_string);
        return;
    }

    let t_verts = mesh_data.solid_verts.as_ptr();
    let t_vert_count = (mesh_data.solid_verts.len() / 3) as i32;
    let t_tris = mesh_data.solid_tris.as_ptr();
    let t_tri_count = (mesh_data.solid_tris.len() / 3) as i32;

    let l_verts = mesh_data.liquid_verts.as_ptr();
    let _l_vert_count = (mesh_data.liquid_verts.len() / 3) as i32;
    let l_tris = mesh_data.liquid_tris.as_ptr();
    let l_tri_count = (mesh_data.liquid_tris.len() / 3) as i32;
    let l_tri_flags = mesh_data.liquid_type.as_ptr();

    // Initialize per-tile config
    let tile_w = config.tile_size + config.border_size * 2;
    let _tile_h = config.tile_size + config.border_size * 2;

    // Build sub-tiles
    let tiles_count = (TILES_PER_MAP * TILES_PER_MAP) as usize;
    let mut poly_meshes: Vec<rc_poly_mesh_t> = vec![std::ptr::null_mut(); tiles_count];
    let mut detail_meshes: Vec<rc_poly_mesh_detail_t> = vec![std::ptr::null_mut(); tiles_count];

    for y in 0..TILES_PER_MAP {
        for x in 0..TILES_PER_MAP {
            let idx = (x + y * TILES_PER_MAP) as usize;

            // Calculate per-tile bounding box
            let mut tile_cfg = create_rc_config_c(config);
            tile_cfg.width = tile_w;
            tile_cfg.height = tile_w; // square tiles

            tile_cfg.bmin[0] = config.bmin[0] + x as f32 * (config.tile_size as f32 * config.cs);
            tile_cfg.bmin[2] = config.bmin[2] + y as f32 * (config.tile_size as f32 * config.cs);
            tile_cfg.bmax[0] = config.bmin[0] + (x + 1) as f32 * (config.tile_size as f32 * config.cs);
            tile_cfg.bmax[2] = config.bmin[2] + (y + 1) as f32 * (config.tile_size as f32 * config.cs);

            // Add border padding
            tile_cfg.bmin[0] -= config.border_size as f32 * config.cs;
            tile_cfg.bmin[2] -= config.border_size as f32 * config.cs;
            tile_cfg.bmax[0] += config.border_size as f32 * config.cs;
            tile_cfg.bmax[2] += config.border_size as f32 * config.cs;

            // Build common tile (Recast pipeline)
            let (pmesh, dmesh) = build_common_tile_recast(
                ctx, tile_string, &tile_cfg,
                t_verts, t_vert_count, t_tris, t_tri_count,
                l_verts, _l_vert_count, l_tris, l_tri_count, l_tri_flags,
            );

            poly_meshes[idx] = pmesh;
            detail_meshes[idx] = dmesh;
        }
    }

    // Collect non-null meshes for merging
    let mut pm_merge: Vec<rc_poly_mesh_t> = Vec::new();
    let mut dm_merge: Vec<rc_poly_mesh_detail_t> = Vec::new();
    for idx in 0..tiles_count {
        if !poly_meshes[idx].is_null() {
            pm_merge.push(poly_meshes[idx]);
            dm_merge.push(detail_meshes[idx]);
        }
    }

    if pm_merge.is_empty() {
        info!("{} No poly meshes to merge", tile_string);
        rc_free_context(ctx);
        return;
    }

    // Merge poly meshes
    let merged_pmesh = rc_alloc_poly_mesh();
    if merged_pmesh.is_null() {
        error!("{} Failed to alloc merged poly mesh!", tile_string);
        rc_free_context(ctx);
        return;
    }
    rc_merge_poly_meshes(ctx, pm_merge.as_mut_ptr(), pm_merge.len() as i32, merged_pmesh);

    let merged_dmesh = rc_alloc_poly_mesh_detail();
    if merged_dmesh.is_null() {
        error!("{} Failed to alloc merged detail mesh!", tile_string);
        rc_free_poly_mesh(merged_pmesh);
        rc_free_context(ctx);
        return;
    }
    rc_merge_poly_mesh_details(ctx, dm_merge.as_mut_ptr(), dm_merge.len() as i32, merged_dmesh);

    // Free sub-tile meshes
    for idx in 0..tiles_count {
        if !poly_meshes[idx].is_null() {
            rc_free_poly_mesh(poly_meshes[idx]);
        }
        if !detail_meshes[idx].is_null() {
            rc_free_poly_mesh_detail(detail_meshes[idx]);
        }
    }

    // Get poly mesh data through accessor
    let mut pm_data: RcPolyMeshDataC = std::mem::zeroed();
    rc_get_poly_mesh_data(merged_pmesh, &mut pm_data);

    // Set polygon flags based on area
    for i in 0..pm_data.npolys as usize {
        let area = pm_data.areas.add(i).read() & NAV_AREA_ALL_MASK;
        if area != 0 {
            if area >= NAV_AREA_MIN_VALUE {
                pm_data.flags.add(i).write(
                    1u16 << (NAV_AREA_MAX_VALUE - area),
                );
            } else {
                pm_data.flags.add(i).write(NAV_GROUND);
            }
        }
    }

    // Get detail mesh data through accessor
    let mut dm_data: RcPolyMeshDetailDataC = std::mem::zeroed();
    rc_get_poly_mesh_detail_data(merged_dmesh, &mut dm_data);

    // Setup dtNavMeshCreateParams
    #[allow(clippy::field_reassign_with_default)]
    let mut params = DtNavMeshCreateParamsC {
        verts: pm_data.verts,
        vert_count: pm_data.nverts,
        polys: pm_data.polys,
        poly_areas: pm_data.areas,
        poly_flags: pm_data.flags,
        poly_count: pm_data.npolys,
        nvp: pm_data.nvp,
        detail_meshes: dm_data.meshes,
        detail_verts: dm_data.verts,
        detail_verts_count: dm_data.nverts,
        detail_tris: dm_data.tris,
        detail_tri_count: dm_data.ntris,
        ..Default::default()
    };

    // Off-mesh connections
    if !mesh_data.off_mesh_connections.is_empty() {
        params.off_mesh_con_verts = mesh_data.off_mesh_connections.as_ptr();
        params.off_mesh_con_count = (mesh_data.off_mesh_connections.len() / 6) as i32;
        params.off_mesh_con_rad = mesh_data.off_mesh_connection_rads.as_ptr();
        params.off_mesh_con_dir = mesh_data.off_mesh_connection_dirs.as_ptr();
        params.off_mesh_con_areas = mesh_data.off_mesh_connections_areas.as_ptr();
        params.off_mesh_con_flags = mesh_data.off_mesh_connections_flags.as_ptr();
    }

    params.walkable_height = BASE_UNIT_DIM * config.walkable_height as f32;
    params.walkable_radius = BASE_UNIT_DIM * config.walkable_radius as f32;
    params.walkable_climb = BASE_UNIT_DIM * config.walkable_climb as f32;

    params.tile_x = (((bmin[0] + bmax[0]) / 2.0 - nav_mesh_params.orig[0]) / GRID_SIZE) as i32;
    params.tile_y = (((bmin[2] + bmax[2]) / 2.0 - nav_mesh_params.orig[2]) / GRID_SIZE) as i32;
    params.bmin = *bmin;
    params.bmax = *bmax;
    params.cs = config.cs;
    params.ch = config.ch;
    params.tile_layer = 0;
    params.build_bv_tree = true;

    // Validate
    if params.nvp > DT_VERTS_PER_POLYGON as i32 {
        error!("{} Invalid verts-per-polygon value!", tile_string);
    } else if params.vert_count >= 0xffff {
        error!("{} Too many vertices!", tile_string);
    } else if params.vert_count == 0 || params.verts.is_null() {
        // No vertices - skip
    } else if params.poly_count == 0
        || params.polys.is_null()
        || params.poly_count == (TILES_PER_MAP * TILES_PER_MAP)
    {
        info!("{} No polygons to build on tile!", tile_string);
    } else if params.detail_meshes.is_null()
        || params.detail_verts.is_null()
        || params.detail_tris.is_null()
    {
        error!("{} No detail mesh to build tile!", tile_string);
    } else {
        // Create navmesh data
        let mut nav_data: *mut u8 = std::ptr::null_mut();
        let mut nav_data_size: i32 = 0;

        if dt_create_nav_mesh_data(&mut params, &mut nav_data, &mut nav_data_size) {
            // Write to file
            let file_name = mmaps_dir.join(format!(
                "{:03}{:02}{:02}.mmtile",
                map_id, tile_y, tile_x
            ));

            if let Some(parent) = file_name.parent() {
                fs::create_dir_all(parent).ok();
            }

            match fs::File::create(&file_name) {
                Ok(mut file) => {
                    // Write MmapTileHeader
                    file.write_u32::<LittleEndian>(MMAP_MAGIC).ok();
                    file.write_u32::<LittleEndian>(DT_NAVMESH_VERSION_CONST).ok();
                    file.write_u32::<LittleEndian>(MMAP_VERSION).ok();
                    file.write_u32::<LittleEndian>(nav_data_size as u32).ok();
                    file.write_u32::<LittleEndian>(if uses_liquids { 1 } else { 0 }).ok();

                    // Write nav data
                    let data_slice =
                        std::slice::from_raw_parts(nav_data, nav_data_size as usize);
                    file.write_all(data_slice).ok();

                    info!(
                        "{} Written to {} [size={}]",
                        tile_string,
                        file_name.display(),
                        nav_data_size
                    );
                }
                Err(e) => {
                    error!(
                        "{} Failed to open {} for writing: {}",
                        tile_string,
                        file_name.display(),
                        e
                    );
                }
            }

            // Free nav data
            dt_free(nav_data as *mut std::ffi::c_void);
        } else {
            error!("{} Failed building navmesh tile!", tile_string);
        }
    }

    // Cleanup
    rc_free_poly_mesh(merged_pmesh);
    rc_free_poly_mesh_detail(merged_dmesh);
    rc_free_context(ctx);

    } // unsafe
}

#[cfg(feature = "recast")]
/// Build a common tile using the Recast pipeline (unsafe FFI)
#[allow(clippy::too_many_arguments)]
unsafe fn build_common_tile_recast(
    ctx: recast_ffi::rc_context_t,
    tile_string: &str,
    tile_cfg: &recast_ffi::RcConfigC,
    t_verts: *const f32,
    t_vert_count: i32,
    t_tris: *const i32,
    t_tri_count: i32,
    l_verts: *const f32,
    l_vert_count: i32,
    l_tris: *const i32,
    l_tri_count: i32,
    l_tri_flags: *const u8,
) -> (recast_ffi::rc_poly_mesh_t, recast_ffi::rc_poly_mesh_detail_t) {
    use recast_ffi::*;
    unsafe {

    let null_result: (rc_poly_mesh_t, rc_poly_mesh_detail_t) =
        (std::ptr::null_mut(), std::ptr::null_mut());

    // Create heightfield
    let solid = rc_alloc_heightfield();
    if solid.is_null()
        || !rc_create_heightfield(
            ctx,
            solid,
            tile_cfg.width,
            tile_cfg.height,
            tile_cfg.bmin.as_ptr(),
            tile_cfg.bmax.as_ptr(),
            tile_cfg.cs,
            tile_cfg.ch,
        )
    {
        rc_free_heightfield(solid);
        return null_result;
    }

    // Mark walkable triangles and rasterize
    if t_tri_count > 0 {
        let mut tri_flags = vec![NAV_AREA_GROUND; t_tri_count as usize];
        rc_clear_unwalkable_triangles(
            ctx,
            tile_cfg.walkable_slope_angle,
            t_verts,
            t_vert_count,
            t_tris,
            t_tri_count,
            tri_flags.as_mut_ptr(),
        );

        // Mark almost-unwalkable (steep) triangles
        rc_mod_almost_unwalkable_triangles(
            50.0,
            t_verts,
            t_tris,
            t_tri_count,
            &mut tri_flags,
        );

        rc_rasterize_triangles(
            ctx,
            t_verts,
            t_vert_count,
            t_tris,
            tri_flags.as_ptr(),
            t_tri_count,
            solid,
            tile_cfg.walkable_climb,
        );
    }

    rc_filter_low_hanging_walkable_obstacles(ctx, tile_cfg.walkable_climb, solid);
    rc_filter_ledge_spans(ctx, tile_cfg.walkable_height, tile_cfg.walkable_climb, solid);
    rc_filter_walkable_low_height_spans(ctx, tile_cfg.walkable_height, solid);

    // Rasterize liquid
    if l_tri_count > 0 && !l_verts.is_null() {
        rc_rasterize_triangles(
            ctx,
            l_verts,
            l_vert_count,
            l_tris,
            l_tri_flags,
            l_tri_count,
            solid,
            tile_cfg.walkable_climb,
        );
    }

    // Compact heightfield
    let chf = rc_alloc_compact_heightfield();
    if chf.is_null()
        || !rc_build_compact_heightfield(
            ctx,
            tile_cfg.walkable_height,
            tile_cfg.walkable_climb,
            solid,
            chf,
        )
    {
        rc_free_heightfield(solid);
        rc_free_compact_heightfield(chf);
        return null_result;
    }

    rc_free_heightfield(solid);

    // Erode walkable area
    if !rc_erode_walkable_area(ctx, tile_cfg.walkable_radius, chf) {
        rc_free_compact_heightfield(chf);
        return null_result;
    }

    // Median filter
    rc_median_filter_walkable_area(ctx, chf);

    // Build distance field
    if !rc_build_distance_field(ctx, chf) {
        rc_free_compact_heightfield(chf);
        return null_result;
    }

    // Build regions
    if !rc_build_regions(
        ctx,
        chf,
        tile_cfg.border_size,
        tile_cfg.min_region_area,
        tile_cfg.merge_region_area,
    ) {
        rc_free_compact_heightfield(chf);
        return null_result;
    }

    // Build contours
    let cset = rc_alloc_contour_set();
    if cset.is_null()
        || !rc_build_contours(
            ctx,
            chf,
            tile_cfg.max_simplification_error,
            tile_cfg.max_edge_len,
            cset,
        )
    {
        rc_free_compact_heightfield(chf);
        rc_free_contour_set(cset);
        return null_result;
    }

    // Build poly mesh
    let pmesh = rc_alloc_poly_mesh();
    if pmesh.is_null()
        || !rc_build_poly_mesh(ctx, cset, tile_cfg.max_verts_per_poly, pmesh)
    {
        rc_free_compact_heightfield(chf);
        rc_free_contour_set(cset);
        rc_free_poly_mesh(pmesh);
        return null_result;
    }

    // Build detail mesh
    let dmesh = rc_alloc_poly_mesh_detail();
    if dmesh.is_null()
        || !rc_build_poly_mesh_detail(
            ctx,
            pmesh,
            chf,
            tile_cfg.detail_sample_dist,
            tile_cfg.detail_sample_max_error,
            dmesh,
        )
    {
        rc_free_compact_heightfield(chf);
        rc_free_contour_set(cset);
        rc_free_poly_mesh(pmesh);
        rc_free_poly_mesh_detail(dmesh);
        return null_result;
    }

    // Free intermediates
    rc_free_compact_heightfield(chf);
    rc_free_contour_set(cset);

    (pmesh, dmesh)

    } // unsafe
}

// ============================================================================
// Helper functions
// ============================================================================

#[cfg(feature = "recast")]
/// Mark triangles with slopes between 50-60 degrees as steep
fn rc_mod_almost_unwalkable_triangles(
    walkable_slope_angle: f32,
    verts: *const f32,
    tris: *const i32,
    tri_count: i32,
    areas: &mut [u8],
) {
    let walkable_thr = (walkable_slope_angle / 180.0 * std::f32::consts::PI).cos();

    for (i, area) in areas.iter_mut().enumerate().take(tri_count as usize) {
        if (*area & 0x3F) != 0 {
            // RC_WALKABLE_AREA check
            unsafe {
                let tri = tris.add(i * 3);
                let i0 = *tri.add(0) as usize;
                let i1 = *tri.add(1) as usize;
                let i2 = *tri.add(2) as usize;

                let v0 = [
                    *verts.add(i0 * 3),
                    *verts.add(i0 * 3 + 1),
                    *verts.add(i0 * 3 + 2),
                ];
                let v1 = [
                    *verts.add(i1 * 3),
                    *verts.add(i1 * 3 + 1),
                    *verts.add(i1 * 3 + 2),
                ];
                let v2 = [
                    *verts.add(i2 * 3),
                    *verts.add(i2 * 3 + 1),
                    *verts.add(i2 * 3 + 2),
                ];

                let e0 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
                let e1 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];

                let mut norm = [
                    e0[1] * e1[2] - e0[2] * e1[1],
                    e0[2] * e1[0] - e0[0] * e1[2],
                    e0[0] * e1[1] - e0[1] * e1[0],
                ];
                let len = (norm[0] * norm[0] + norm[1] * norm[1] + norm[2] * norm[2]).sqrt();
                if len > 0.0 {
                    norm[0] /= len;
                    norm[1] /= len;
                    norm[2] /= len;
                }

                if norm[1] <= walkable_thr {
                    *area = NAV_AREA_GROUND_STEEP;
                }
            }
        }
    }
}

#[cfg(feature = "recast")]
/// Create a C-compatible rcConfig struct from our config
fn create_rc_config_c(config: &RcConfig) -> recast_ffi::RcConfigC {
    recast_ffi::RcConfigC {
        width: config.width,
        height: config.height,
        tile_size: config.tile_size,
        border_size: config.border_size,
        cs: config.cs,
        ch: config.ch,
        bmin: config.bmin,
        bmax: config.bmax,
        walkable_slope_angle: config.walkable_slope_angle,
        walkable_height: config.walkable_height,
        walkable_climb: config.walkable_climb,
        walkable_radius: config.walkable_radius,
        max_edge_len: config.max_edge_len,
        max_simplification_error: config.max_simplification_error,
        min_region_area: config.min_region_area,
        merge_region_area: config.merge_region_area,
        max_verts_per_poly: config.max_verts_per_poly,
        detail_sample_dist: config.detail_sample_dist,
        detail_sample_max_error: config.detail_sample_max_error,
        liquid_flag_merge_threshold: 0.0,
    }
}

fn get_loop_vars(portion: Spot) -> (usize, usize, usize) {
    match portion {
        Spot::Entire => (0, V8_SIZE_SQ, 1),
        Spot::Top => (0, V8_SIZE, 1),
        Spot::Left => (0, V8_SIZE_SQ - V8_SIZE + 1, V8_SIZE),
        Spot::Right => (V8_SIZE - 1, V8_SIZE_SQ, V8_SIZE),
        Spot::Bottom => (V8_SIZE_SQ - V8_SIZE, V8_SIZE_SQ, 1),
    }
}

fn get_height_coord(index: usize, grid: Grid, x_offset: f32, y_offset: f32, v: &[f32]) -> [f32; 3] {
    match grid {
        Grid::V9 => [
            -(x_offset + (index % V9_SIZE) as f32 * GRID_PART_SIZE),
            -(y_offset + (index / V9_SIZE) as f32 * GRID_PART_SIZE),
            v[index],
        ],
        Grid::V8 => [
            -(x_offset + (index % V8_SIZE) as f32 * GRID_PART_SIZE + GRID_PART_SIZE / 2.0),
            -(y_offset + (index / V8_SIZE) as f32 * GRID_PART_SIZE + GRID_PART_SIZE / 2.0),
            v[index],
        ],
    }
}

fn get_liquid_coord(index: usize, index2: usize, x_offset: f32, y_offset: f32, v: &[f32]) -> [f32; 3] {
    [
        -(x_offset + (index % V9_SIZE) as f32 * GRID_PART_SIZE),
        -(y_offset + (index / V9_SIZE) as f32 * GRID_PART_SIZE),
        v[index2],
    ]
}

fn get_height_triangle(square: usize, triangle: Spot, liquid: bool) -> [i32; 3] {
    let row_offset = (square / V8_SIZE) as i32;
    let sq = square as i32;

    if !liquid {
        match triangle {
            Spot::Top => [
                sq + row_offset,
                sq + 1 + row_offset,
                V9_SIZE_SQ as i32 + sq,
            ],
            Spot::Left => [
                sq + row_offset,
                V9_SIZE_SQ as i32 + sq,
                sq + V9_SIZE as i32 + row_offset,
            ],
            Spot::Right => [
                sq + 1 + row_offset,
                sq + V9_SIZE as i32 + 1 + row_offset,
                V9_SIZE_SQ as i32 + sq,
            ],
            Spot::Bottom => [
                V9_SIZE_SQ as i32 + sq,
                sq + V9_SIZE as i32 + 1 + row_offset,
                sq + V9_SIZE as i32 + row_offset,
            ],
            _ => [0, 0, 0],
        }
    } else {
        match triangle {
            Spot::Top => [
                sq + row_offset,
                sq + 1 + row_offset,
                sq + V9_SIZE as i32 + 1 + row_offset,
            ],
            Spot::Bottom => [
                sq + row_offset,
                sq + V9_SIZE as i32 + 1 + row_offset,
                sq + V9_SIZE as i32 + row_offset,
            ],
            _ => [0, 0, 0],
        }
    }
}

fn is_hole(square: usize, holes: &[[u16; 16]; 16]) -> bool {
    let row = square / 128;
    let col = square % 128;
    let cell_row = row / 8;
    let cell_col = col / 8;
    let hole_row = row % 8 / 2;
    let hole_col = (square - (row * 128 + cell_col * 8)) / 2;

    if cell_row >= 16 || cell_col >= 16 || hole_col >= 4 || hole_row >= 4 {
        return false;
    }

    let hole = holes[cell_row][cell_col];
    (hole & HOLETAB_H[hole_col] & HOLETAB_V[hole_row]) != 0
}

fn get_liquid_type(square: usize, liquid_flags: &[[u8; 16]; 16]) -> u8 {
    let row = square / 128;
    let col = square % 128;
    let cell_row = row / 8;
    let cell_col = col / 8;

    if cell_row >= 16 || cell_col >= 16 {
        return MAP_LIQUID_TYPE_NO_WATER;
    }

    liquid_flags[cell_row][cell_col]
}

fn is_transport_map(map_id: u32) -> bool {
    matches!(map_id, 582 | 584 | 586 | 587 | 588 | 589 | 590 | 591 | 593)
}

fn pack_tile_id(x: u32, y: u32) -> u32 {
    (x << 16) | y
}

fn unpack_tile_id(packed: u32) -> (u32, u32) {
    (packed >> 16, packed & 0xFFFF)
}

fn get_tile_bounds(tile_x: u32, tile_y: u32, verts: &[f32], vert_count: usize) -> ([f32; 3], [f32; 3]) {
    let mut bmin: [f32; 3];
    let mut bmax: [f32; 3];

    if vert_count > 0 {
        bmin = [f32::MAX; 3];
        bmax = [f32::MIN; 3];
        for i in 0..vert_count {
            let x = verts[i * 3];
            let y = verts[i * 3 + 1];
            let z = verts[i * 3 + 2];
            bmin[0] = bmin[0].min(x);
            bmin[1] = bmin[1].min(y);
            bmin[2] = bmin[2].min(z);
            bmax[0] = bmax[0].max(x);
            bmax[1] = bmax[1].max(y);
            bmax[2] = bmax[2].max(z);
        }
    } else {
        bmin = [0.0, f32::MIN, 0.0];
        bmax = [0.0, f32::MAX, 0.0];
    }

    // Width and depth from tile coordinates
    bmax[0] = (32 - tile_x as i32) as f32 * GRID_SIZE;
    bmax[2] = (32 - tile_y as i32) as f32 * GRID_SIZE;
    bmin[0] = bmax[0] - GRID_SIZE;
    bmin[2] = bmax[2] - GRID_SIZE;

    (bmin, bmax)
}

fn clean_vertices(verts: &mut Vec<f32>, tris: &mut [i32]) {
    if tris.is_empty() {
        return;
    }

    let mut vert_map: BTreeMap<i32, i32> = BTreeMap::new();

    // Collect vertex indices from triangles
    for &t in tris.iter() {
        vert_map.entry(t).or_insert(0);
    }

    // Build clean vertex list
    let mut clean_verts: Vec<f32> = Vec::new();
    for (count, (_index, new_idx)) in vert_map.iter_mut().enumerate() {
        let idx = *_index as usize;
        *new_idx = count as i32;
        if idx * 3 + 2 < verts.len() {
            clean_verts.push(verts[idx * 3]);
            clean_verts.push(verts[idx * 3 + 1]);
            clean_verts.push(verts[idx * 3 + 2]);
        }
    }

    *verts = clean_verts;

    // Update triangle indices
    for t in tris.iter_mut() {
        if let Some(&new_idx) = vert_map.get(t) {
            *t = new_idx;
        }
    }
}

fn get_tile_config(
    config_json: &Option<serde_json::Value>,
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
) -> MmapConfig {
    let mut config = MmapConfig::default();

    if let Some(json) = config_json {
        let key = map_id.to_string();
        if let Some(map_config) = json.get(&key)
            && let Ok(overrides) = serde_json::from_value::<MmapConfig>(map_config.clone())
        {
            config = overrides;
        }
    }

    config
}

// ============================================================================
// VMap model reading helpers
// ============================================================================

/// Minimal model spawn data (from .vmtree file)
struct ModelSpawnData {
    name: String,
    pos: [f32; 3],
    rot: [f32; 3],
    scale: f32,
    flags: u32,
}

/// Group data from a .vmo file
struct WorldModelData {
    groups: Vec<GroupData>,
}

struct GroupData {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    liquid: Option<VmapLiquidData>,
}

fn read_model_spawn(cursor: &mut std::io::Cursor<&Vec<u8>>) -> Option<ModelSpawnData> {
    let flags = read_u32_le(cursor);
    let _adt_id = read_u16_le(cursor);
    let _id = read_u32_le(cursor);
    let pos = [read_f32_le(cursor), read_f32_le(cursor), read_f32_le(cursor)];
    let rot = [read_f32_le(cursor), read_f32_le(cursor), read_f32_le(cursor)];
    let scale = read_f32_le(cursor);

    // Read bounds if flag set
    if (flags & 4) != 0 {
        // MOD_HAS_BOUND
        for _ in 0..6 {
            read_f32_le(cursor);
        }
    }

    // Read name
    let name_len = read_u32_le(cursor) as usize;
    let mut name_bytes = vec![0u8; name_len];
    cursor.read_exact(&mut name_bytes).ok()?;
    let name = String::from_utf8_lossy(&name_bytes).trim_end_matches('\0').to_string();

    Some(ModelSpawnData {
        name,
        pos,
        rot,
        scale,
        flags,
    })
}

fn load_world_model(path: &Path) -> Option<WorldModelData> {
    let data = fs::read(path).ok()?;
    if data.len() < 12 {
        return None;
    }

    let mut cursor = std::io::Cursor::new(&data);
    let mut magic = [0u8; 8];
    cursor.read_exact(&mut magic).ok()?;

    // Read header
    let _root_wmo_id = read_u32_le(&mut cursor);
    let n_groups = read_u32_le(&mut cursor);
    let _model_flags = read_u32_le(&mut cursor);

    let mut groups = Vec::new();

    // Read group BIH (skip)
    // bounds: 6 floats, tree_size: u32, tree[tree_size], obj_count: u32, objs[obj_count]
    for _ in 0..6 {
        read_f32_le(&mut cursor);
    }
    let tree_size = read_u32_le(&mut cursor);
    let pos = cursor.position() + tree_size as u64 * 4;
    cursor.set_position(pos);
    let obj_count = read_u32_le(&mut cursor);
    let pos = cursor.position() + obj_count as u64 * 4;
    cursor.set_position(pos);

    // Read each group
    for _ in 0..n_groups {
        let group = read_group_model(&mut cursor)?;
        groups.push(group);
    }

    Some(WorldModelData { groups })
}

fn read_group_model(cursor: &mut std::io::Cursor<&Vec<u8>>) -> Option<GroupData> {
    let _mogp_flags = read_u32_le(cursor);
    let _group_wmo_id = read_u32_le(cursor);

    // Read bounds
    for _ in 0..6 {
        read_f32_le(cursor);
    }

    // Read mesh BIH (skip)
    for _ in 0..6 {
        read_f32_le(cursor);
    }
    let tree_size = read_u32_le(cursor);
    let pos = cursor.position() + tree_size as u64 * 4;
    cursor.set_position(pos);
    let obj_count = read_u32_le(cursor);
    let pos = cursor.position() + obj_count as u64 * 4;
    cursor.set_position(pos);

    // Read vertices
    let mut chunk_magic = [0u8; 4];
    cursor.read_exact(&mut chunk_magic).ok()?;
    // "VERT"
    let n_verts = read_u32_le(cursor);
    let mut vertices = Vec::with_capacity(n_verts as usize);
    for _ in 0..n_verts {
        let x = read_f32_le(cursor);
        let y = read_f32_le(cursor);
        let z = read_f32_le(cursor);
        vertices.push([x, y, z]);
    }

    // Read triangles
    cursor.read_exact(&mut chunk_magic).ok()?;
    // "TRIM"
    let n_tris = read_u32_le(cursor);
    let mut triangles = Vec::with_capacity(n_tris as usize);
    for _ in 0..n_tris {
        let i0 = read_u32_le(cursor);
        let i1 = read_u32_le(cursor);
        let i2 = read_u32_le(cursor);
        triangles.push([i0, i1, i2]);
    }

    // Read mesh tree BIH (skip over)
    for _ in 0..6 {
        read_f32_le(cursor);
    }
    let tree_size2 = read_u32_le(cursor);
    let pos = cursor.position() + tree_size2 as u64 * 4;
    cursor.set_position(pos);
    let obj_count2 = read_u32_le(cursor);
    let pos = cursor.position() + obj_count2 as u64 * 4;
    cursor.set_position(pos);

    // Read liquid
    let has_liquid = read_u32_le(cursor);
    let liquid = if has_liquid != 0 {
        let tiles_x = read_u32_le(cursor);
        let tiles_y = read_u32_le(cursor);
        let corner = [read_f32_le(cursor), read_f32_le(cursor), read_f32_le(cursor)];
        let liq_type = read_u32_le(cursor);
        let verts_x = tiles_x + 1;
        let verts_y = tiles_y + 1;
        let data_size = (verts_x * verts_y) as usize;
        let mut heights = vec![0.0f32; data_size];
        for h in heights.iter_mut() {
            *h = read_f32_le(cursor);
        }
        let flags_size = (tiles_x * tiles_y) as usize;
        let mut flags = vec![0u8; flags_size];
        cursor.read_exact(&mut flags).ok()?;

        Some(VmapLiquidData {
            tiles_x,
            tiles_y,
            corner,
            liq_type,
            heights,
            flags,
        })
    } else {
        None
    };

    Some(GroupData {
        vertices,
        triangles,
        liquid,
    })
}

// ============================================================================
// Math helpers
// ============================================================================

fn matrix3_from_euler_xyz(x: f32, y: f32, z: f32) -> [[f32; 3]; 3] {
    let cx = x.cos();
    let sx = x.sin();
    let cy = y.cos();
    let sy = y.sin();
    let cz = z.cos();
    let sz = z.sin();

    [
        [cy * cz, -cy * sz, sy],
        [cz * sx * sy + cx * sz, cx * cz - sx * sy * sz, -cy * sx],
        [-cx * cz * sy + sx * sz, cz * sx + cx * sy * sz, cx * cy],
    ]
}

fn mat3_mul_vec3(m: &[[f32; 3]; 3], v: &[f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

// ============================================================================
// Binary read helpers
// ============================================================================

fn read_u8<R: Read>(r: &mut R) -> u8 {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf).unwrap_or_default();
    buf[0]
}

fn read_u16_le<R: Read>(r: &mut R) -> u16 {
    r.read_u16::<LittleEndian>().unwrap_or(0)
}

fn read_u32_le<R: Read>(r: &mut R) -> u32 {
    r.read_u32::<LittleEndian>().unwrap_or(0)
}

fn read_f32_le<R: Read>(r: &mut R) -> f32 {
    r.read_f32::<LittleEndian>().unwrap_or(0.0)
}

// ============================================================================
// Public API - called from main.rs
// ============================================================================

pub fn run_movemap_gen(args: &super::MoveMapGenArgs) -> anyhow::Result<()> {
    let workdir = Path::new(&args.workdir);

    // Resolve directory paths: use custom overrides if provided, else fall back to workdir/<name>
    let maps_dir = match args.maps_dir {
        Some(ref p) => PathBuf::from(p),
        None => workdir.join("maps"),
    };
    let vmaps_dir = match args.vmaps_dir {
        Some(ref p) => PathBuf::from(p),
        None => workdir.join("vmaps"),
    };
    let mmaps_dir = match args.mmaps_dir {
        Some(ref p) => PathBuf::from(p),
        None => workdir.join("mmaps"),
    };

    // Validate input directories exist
    if !maps_dir.exists() {
        bail!("Maps directory does not exist: {}", maps_dir.display());
    }
    if !vmaps_dir.exists() {
        bail!("VMaps directory does not exist: {}", vmaps_dir.display());
    }

    // Ensure mmaps output directory exists
    fs::create_dir_all(&mmaps_dir)
        .context("Failed to create mmaps directory")?;

    info!(
        "Directories: maps='{}' vmaps='{}' mmaps='{}'",
        maps_dir.display(),
        vmaps_dir.display(),
        mmaps_dir.display(),
    );

    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));

    let config_path = Path::new(&args.config_input);
    let off_mesh_path = Path::new(&args.off_mesh_input);

    let mut builder = MapBuilder::new(
        if config_path.exists() { Some(config_path) } else { None },
        threads,
        args.skip_liquid,
        args.skip_continents,
        args.skip_junk_maps,
        args.skip_battlegrounds,
        args.debug_output,
        if off_mesh_path.exists() { Some(off_mesh_path) } else { None },
        &maps_dir,
        &vmaps_dir,
        &mmaps_dir,
    );

    if let Some(ref tile) = args.tile {
        if let Some(&map_id) = args.map_ids.first() {
            info!("Building single tile: map={}, tile={},{}", map_id, tile.x, tile.y);
            builder.build_single_tile(map_id, tile.x as u32, tile.y as u32);
        } else {
            bail!("Map ID required for --tile option");
        }
    } else {
        let map_ids: Vec<u32> = args.map_ids.clone();
        builder.build_maps(&map_ids);
    }

    if args.build_game_objects {
        builder.build_transports();
    }

    info!("MoveMapGen complete.");
    Ok(())
}
