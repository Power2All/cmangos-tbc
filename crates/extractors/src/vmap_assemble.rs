use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::VmapAssembleArgs;

const VMAP_MAGIC: &str = "VMAP_7.0";
const RAW_VMAP_MAGIC: &str = "VMAPs05";
const GAMEOBJECT_MODELS: &str = "temp_gameobject_models";

const MOD_M2: u32 = 1;
const MOD_WORLDSPAWN: u32 = 1 << 1;
const MOD_HAS_BOUND: u32 = 1 << 2;

const WORLDSPAWN_OFFSET: f32 = 533.33333 * 32.0;

#[derive(Clone, Copy, Debug, Default)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn min(self, other: Self) -> Self {
        Self::new(self.x.min(other.x), self.y.min(other.y), self.z.min(other.z))
    }

    fn max(self, other: Self) -> Self {
        Self::new(self.x.max(other.x), self.y.max(other.y), self.z.max(other.z))
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    fn scale(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AaBox {
    min: Vec3,
    max: Vec3,
}

impl AaBox {
    fn from_point(p: Vec3) -> Self {
        Self { min: p, max: p }
    }

    fn merge(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn add(self, v: Vec3) -> Self {
        Self {
            min: self.min.add(v),
            max: self.max.add(v),
        }
    }
}

#[derive(Clone, Debug)]
struct ModelSpawn {
    flags: u32,
    adt_id: u16,
    id: u32,
    pos: Vec3,
    rot: Vec3,
    scale: f32,
    bound: Option<AaBox>,
    name: String,
}

impl ModelSpawn {
    fn read_from<R: Read>(reader: &mut R) -> anyhow::Result<Option<Self>> {
        let flags = match reader.read_u32::<LittleEndian>() {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let adt_id = reader.read_u16::<LittleEndian>()?;
        let id = reader.read_u32::<LittleEndian>()?;
        let pos = read_vec3(reader)?;
        let rot = read_vec3(reader)?;
        let scale = reader.read_f32::<LittleEndian>()?;

        let bound = if (flags & MOD_HAS_BOUND) != 0 {
            let min = read_vec3(reader)?;
            let max = read_vec3(reader)?;
            Some(AaBox { min, max })
        } else {
            None
        };

        let name_len = reader.read_u32::<LittleEndian>()? as usize;
        if name_len > 500 {
            anyhow::bail!("ModelSpawn name length too large: {}", name_len);
        }
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let name = String::from_utf8_lossy(&name_buf).to_string();

        Ok(Some(Self {
            flags,
            adt_id,
            id,
            pos,
            rot,
            scale,
            bound,
            name,
        }))
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        writer.write_u32::<LittleEndian>(self.flags)?;
        writer.write_u16::<LittleEndian>(self.adt_id)?;
        writer.write_u32::<LittleEndian>(self.id)?;
        write_vec3(writer, self.pos)?;
        write_vec3(writer, self.rot)?;
        writer.write_f32::<LittleEndian>(self.scale)?;
        if let Some(bound) = self.bound {
            write_vec3(writer, bound.min)?;
            write_vec3(writer, bound.max)?;
        }
        writer.write_u32::<LittleEndian>(self.name.len() as u32)?;
        writer.write_all(self.name.as_bytes())?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct MeshTriangle {
    idx0: u32,
    idx1: u32,
    idx2: u32,
}

#[derive(Clone, Debug)]
struct WmoLiquid {
    tiles_x: u32,
    tiles_y: u32,
    corner: Vec3,
    liquid_type: u32,
    heights: Vec<f32>,
    flags: Vec<u8>,
}

impl WmoLiquid {
    fn write_to<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        writer.write_u32::<LittleEndian>(self.tiles_x)?;
        writer.write_u32::<LittleEndian>(self.tiles_y)?;
        write_vec3(writer, self.corner)?;
        writer.write_u32::<LittleEndian>(self.liquid_type)?;
        let height_size = (self.tiles_x + 1) * (self.tiles_y + 1);
        for i in 0..height_size as usize {
            let value = self.heights.get(i).copied().unwrap_or(0.0);
            writer.write_f32::<LittleEndian>(value)?;
        }
        let flag_size = (self.tiles_x * self.tiles_y) as usize;
        let mut flags = self.flags.clone();
        flags.resize(flag_size, 0);
        writer.write_all(&flags)?;
        Ok(())
    }

    fn file_size(&self) -> u32 {
        let height_size = (self.tiles_x + 1) * (self.tiles_y + 1);
        2 * std::mem::size_of::<u32>() as u32
            + std::mem::size_of::<Vec3>() as u32
            + height_size * std::mem::size_of::<f32>() as u32
            + (self.tiles_x * self.tiles_y) as u32
    }
}

#[derive(Clone, Debug)]
struct RawGroup {
    mogp_flags: u32,
    group_wmo_id: u32,
    bounds: AaBox,
    liquid_flags: u32,
    triangles: Vec<MeshTriangle>,
    vertices: Vec<Vec3>,
    liquid: Option<WmoLiquid>,
}

#[derive(Clone, Debug)]
struct RawModel {
    root_wmo_id: u32,
    groups: Vec<RawGroup>,
}

#[derive(Default)]
struct MapSpawns {
    unique_entries: BTreeMap<u32, ModelSpawn>,
    tile_entries: Vec<(u32, u32)>,
}

pub fn run_vmap_assemble(args: VmapAssembleArgs) -> anyhow::Result<()> {
    tracing::info!(
        "VMap assembler: raw='{}' output='{}'",
        args.raw_data_dir,
        args.output_dir
    );

    let raw_dir = Path::new(&args.raw_data_dir);
    if !raw_dir.exists() {
        anyhow::bail!("Raw data directory does not exist: {}", args.raw_data_dir);
    }

    let output_dir = PathBuf::from(&args.output_dir);
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir)?;
    }

    let mut map_data: BTreeMap<u32, MapSpawns> = BTreeMap::new();
    read_map_spawns(raw_dir, &mut map_data)?;

    let mut spawned_model_files = HashSet::new();
    for (map_id, spawns) in &mut map_data {
        tracing::info!("Calculating model bounds for map {}...", map_id);
        let mut missing = Vec::new();
        for (spawn_id, spawn) in spawns.unique_entries.iter_mut() {
            if !raw_dir.join(&spawn.name).exists() {
                tracing::warn!(
                    "Missing raw model file for spawn {} (map {}): {}",
                    spawn_id,
                    map_id,
                    spawn.name
                );
                missing.push(*spawn_id);
                continue;
            }
            if (spawn.flags & MOD_M2) != 0 {
                if let Err(err) = calculate_transformed_bound(raw_dir, spawn) {
                    tracing::warn!(
                        "Failed to calculate bounds for spawn {} (map {}): {}",
                        spawn_id,
                        map_id,
                        err
                    );
                    missing.push(*spawn_id);
                    continue;
                }
            } else if (spawn.flags & MOD_WORLDSPAWN) != 0 {
                if let Some(bound) = spawn.bound {
                    let offset = Vec3::new(WORLDSPAWN_OFFSET, WORLDSPAWN_OFFSET, 0.0);
                    spawn.bound = Some(bound.add(offset));
                }
            }
            spawned_model_files.insert(spawn.name.clone());
        }

        if !missing.is_empty() {
            for spawn_id in &missing {
                spawns.unique_entries.remove(spawn_id);
            }
            spawns
                .tile_entries
                .retain(|(_, spawn_id)| !missing.contains(spawn_id));
        }

        write_map_files(&output_dir, *map_id, spawns)?;
    }

    export_gameobject_models(raw_dir, &output_dir, &mut spawned_model_files)?;

    tracing::info!("Converting Model Files");
    for model in spawned_model_files {
        tracing::info!("Converting {}", model);
        if let Err(err) = convert_raw_file(raw_dir, &output_dir, &model) {
            tracing::warn!("Skipping model {} due to error: {}", model, err);
        }
    }

    Ok(())
}

fn read_map_spawns(raw_dir: &Path, map_data: &mut BTreeMap<u32, MapSpawns>) -> anyhow::Result<()> {
    let path = raw_dir.join("dir_bin");
    let file = File::open(&path).with_context(|| format!("Could not read {}", path.display()))?;
    let mut reader = BufReader::new(file);

    loop {
        let map_id = match reader.read_u32::<LittleEndian>() {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        };
        let tile_x = reader.read_u32::<LittleEndian>()?;
        let tile_y = reader.read_u32::<LittleEndian>()?;

        let Some(spawn) = ModelSpawn::read_from(&mut reader)? else {
            break;
        };

        let entry = map_data.entry(map_id).or_default();
        let spawn_id = spawn.id;
        entry.unique_entries.entry(spawn_id).or_insert(spawn);
        entry
            .tile_entries
            .push((pack_tile_id(tile_x, tile_y), spawn_id));
    }

    Ok(())
}

fn write_map_files(output_dir: &Path, map_id: u32, spawns: &MapSpawns) -> anyhow::Result<()> {
    let mut map_spawns = Vec::new();
    for spawn in spawns.unique_entries.values() {
        if spawn.bound.is_none() {
            anyhow::bail!("Spawn {} has no bounds", spawn.name);
        }
        map_spawns.push(spawn);
    }

    let (bih, node_index) = build_map_bih(&map_spawns);
    let global_tile_id = pack_tile_id(65, 65);
    let has_global = spawns
        .tile_entries
        .iter()
        .any(|(tile_id, _)| *tile_id == global_tile_id);
    let is_tiled = if has_global { 0u8 } else { 1u8 };

    let map_file = output_dir.join(format!("{:03}.vmtree", map_id));
    let mut out = File::create(&map_file)?;
    out.write_all(VMAP_MAGIC.as_bytes())?;
    out.write_u8(is_tiled)?;
    out.write_all(b"NODE")?;
    bih.write_to(&mut out)?;
    out.write_all(b"GOBJ")?;

    if has_global {
        for (tile_id, spawn_id) in &spawns.tile_entries {
            if *tile_id != global_tile_id {
                continue;
            }
            let spawn = spawns
                .unique_entries
                .get(spawn_id)
                .context("Missing spawn for global tile")?;
            spawn.write_to(&mut out)?;
            let idx = node_index
                .get(spawn_id)
                .copied()
                .unwrap_or(0);
            out.write_u32::<LittleEndian>(idx)?;
        }
    }

    let mut tile_entries = spawns.tile_entries.clone();
    tile_entries.sort_by_key(|entry| entry.0);

    let mut idx = 0usize;
    while idx < tile_entries.len() {
        let tile_id = tile_entries[idx].0;
        let start = idx;
        while idx < tile_entries.len() && tile_entries[idx].0 == tile_id {
            idx += 1;
        }
        // Filter out MOD_WORLDSPAWN entries (matching C++ behavior)
        let non_worldspawn: Vec<_> = tile_entries[start..idx]
            .iter()
            .filter(|e| {
                spawns
                    .unique_entries
                    .get(&e.1)
                    .map_or(false, |s| (s.flags & MOD_WORLDSPAWN) == 0)
            })
            .collect();
        if non_worldspawn.is_empty() {
            continue;
        }
        let count = non_worldspawn.len() as u32;
        let (tile_x, tile_y) = unpack_tile_id(tile_id);
        let tile_file = output_dir.join(format!("{:03}_{:02}_{:02}.vmtile", map_id, tile_x, tile_y));
        let mut tile_out = File::create(&tile_file)?;
        tile_out.write_all(VMAP_MAGIC.as_bytes())?;
        tile_out.write_u32::<LittleEndian>(count)?;
        for entry in &non_worldspawn {
            let spawn = spawns
                .unique_entries
                .get(&entry.1)
                .context("Missing spawn for tile entry")?;
            spawn.write_to(&mut tile_out)?;
            let idx = node_index.get(&entry.1).copied().unwrap_or(0);
            tile_out.write_u32::<LittleEndian>(idx)?;
        }
    }

    Ok(())
}

fn export_gameobject_models(
    raw_dir: &Path,
    output_dir: &Path,
    spawned_model_files: &mut HashSet<String>,
) -> anyhow::Result<()> {
    let src = raw_dir.join(GAMEOBJECT_MODELS);
    if !src.exists() {
        return Ok(());
    }

    let mut src_file = BufReader::new(File::open(&src)?);
    let dest = output_dir.join(GAMEOBJECT_MODELS);
    let mut dest_file = File::create(&dest)?;

    loop {
        let display_id = match src_file.read_u32::<LittleEndian>() {
            Ok(value) => value,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        };
        let name_len = src_file.read_u32::<LittleEndian>()? as usize;
        if name_len > 500 {
            anyhow::bail!("Gameobject model name length too large: {}", name_len);
        }
        let mut name_buf = vec![0u8; name_len];
        src_file.read_exact(&mut name_buf)?;
        let name = String::from_utf8_lossy(&name_buf).to_string();

        let raw_model = match read_raw_model(&raw_dir.join(&name)) {
            Ok(model) => model,
            Err(err) => {
                tracing::warn!("Skipping gameobject model {}: {}", name, err);
                continue;
            }
        };
        let bounds = compute_model_bounds(&raw_model).unwrap_or_else(|| {
            let zero = Vec3::new(0.0, 0.0, 0.0);
            AaBox { min: zero, max: zero }
        });

        spawned_model_files.insert(name.clone());

        dest_file.write_u32::<LittleEndian>(display_id)?;
        dest_file.write_u32::<LittleEndian>(name_len as u32)?;
        dest_file.write_all(&name_buf)?;
        write_vec3(&mut dest_file, bounds.min)?;
        write_vec3(&mut dest_file, bounds.max)?;
    }

    Ok(())
}

fn convert_raw_file(raw_dir: &Path, output_dir: &Path, name: &str) -> anyhow::Result<()> {
    let raw_model = read_raw_model(&raw_dir.join(name))?;
    let vmo_path = output_dir.join(format!("{}.vmo", name));
    let mut out = File::create(&vmo_path)?;

    out.write_all(VMAP_MAGIC.as_bytes())?;
    out.write_all(b"WMOD")?;
    out.write_u32::<LittleEndian>((std::mem::size_of::<u32>() * 2) as u32)?;
    out.write_u32::<LittleEndian>(raw_model.root_wmo_id)?;

    if !raw_model.groups.is_empty() {
        out.write_all(b"GMOD")?;
        out.write_u32::<LittleEndian>(raw_model.groups.len() as u32)?;
        for group in &raw_model.groups {
            write_group_model(&mut out, group)?;
        }
        out.write_all(b"GBIH")?;
        let group_bih = build_group_bih(&raw_model.groups);
        group_bih.write_to(&mut out)?;
    }

    Ok(())
}

fn write_group_model<W: Write>(writer: &mut W, group: &RawGroup) -> anyhow::Result<()> {
    write_vec3(writer, group.bounds.min)?;
    write_vec3(writer, group.bounds.max)?;
    writer.write_u32::<LittleEndian>(group.mogp_flags)?;
    writer.write_u32::<LittleEndian>(group.group_wmo_id)?;

    writer.write_all(b"VERT")?;
    let count = group.vertices.len() as u32;
    let chunk_size = std::mem::size_of::<u32>() as u32 + count * std::mem::size_of::<Vec3>() as u32;
    writer.write_u32::<LittleEndian>(chunk_size)?;
    writer.write_u32::<LittleEndian>(count)?;
    if count == 0 {
        return Ok(());
    }
    for v in &group.vertices {
        write_vec3(writer, *v)?;
    }

    writer.write_all(b"TRIM")?;
    let tcount = group.triangles.len() as u32;
    let chunk_size = std::mem::size_of::<u32>() as u32
        + tcount * std::mem::size_of::<MeshTriangle>() as u32;
    writer.write_u32::<LittleEndian>(chunk_size)?;
    writer.write_u32::<LittleEndian>(tcount)?;
    for tri in &group.triangles {
        writer.write_u32::<LittleEndian>(tri.idx0)?;
        writer.write_u32::<LittleEndian>(tri.idx1)?;
        writer.write_u32::<LittleEndian>(tri.idx2)?;
    }

    writer.write_all(b"MBIH")?;
    let mesh_bih = build_mesh_bih(group);
    mesh_bih.write_to(writer)?;

    writer.write_all(b"LIQU")?;
    let liquid_size = group.liquid.as_ref().map(|liq| liq.file_size()).unwrap_or(0);
    writer.write_u32::<LittleEndian>(liquid_size)?;
    if let Some(liquid) = &group.liquid {
        liquid.write_to(writer)?;
    }

    Ok(())
}

fn read_raw_model(path: &Path) -> anyhow::Result<RawModel> {
    let file = File::open(path).with_context(|| format!("Missing raw model file: {}", path.display()))?;
    let mut data = Vec::new();
    let mut reader = BufReader::new(file);
    reader.read_to_end(&mut data)?;

    let mut last_err = None;
    for header_len in [8usize, 7usize] {
        if header_len == 8 {
            if data.len() < 8 || data[7] != 0 {
                continue;
            }
        }
        match parse_raw_model_with_header(&data, header_len) {
            Ok(model) => return Ok(model),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Invalid raw vmap file: {}", path.display())))
}

fn parse_raw_model_with_header(data: &[u8], header_len: usize) -> anyhow::Result<RawModel> {
    if data.len() < header_len + 12 {
        anyhow::bail!("Raw vmap file too small");
    }

    if &data[..7] != RAW_VMAP_MAGIC.as_bytes() {
        anyhow::bail!("Invalid raw vmap magic");
    }

    let mut cursor = std::io::Cursor::new(data);
    cursor.set_position(header_len as u64);

    let _temp_vectors = cursor.read_u32::<LittleEndian>()?;
    let group_count = cursor.read_u32::<LittleEndian>()?;
    let root_wmo_id = cursor.read_u32::<LittleEndian>()?;

    let mut groups = Vec::with_capacity(group_count as usize);
    for _ in 0..group_count {
        groups.push(read_raw_group(&mut cursor)?);
    }

    Ok(RawModel { root_wmo_id, groups })
}

fn read_raw_group<R: Read>(reader: &mut R) -> anyhow::Result<RawGroup> {
    let mogp_flags = reader.read_u32::<LittleEndian>()?;
    let group_wmo_id = reader.read_u32::<LittleEndian>()?;
    let min = read_vec3(reader)?;
    let max = read_vec3(reader)?;
    let bounds = AaBox { min, max };
    let liquid_flags = reader.read_u32::<LittleEndian>()?;

    read_chunk(reader, b"GRP ")?;
    let _block_size = reader.read_i32::<LittleEndian>()?;
    let branches = reader.read_u32::<LittleEndian>()?;
    for _ in 0..branches {
        let _ = reader.read_u32::<LittleEndian>()?;
    }

    read_chunk(reader, b"INDX")?;
    let _block_size = reader.read_i32::<LittleEndian>()?;
    let nindexes = reader.read_u32::<LittleEndian>()?;
    let mut indices = Vec::with_capacity(nindexes as usize);
    for _ in 0..nindexes {
        indices.push(reader.read_u16::<LittleEndian>()? as u32);
    }
    let mut triangles = Vec::with_capacity(indices.len() / 3);
    for chunk in indices.chunks(3) {
        if chunk.len() == 3 {
            triangles.push(MeshTriangle {
                idx0: chunk[0],
                idx1: chunk[1],
                idx2: chunk[2],
            });
        }
    }

    read_chunk(reader, b"VERT")?;
    let _block_size = reader.read_i32::<LittleEndian>()?;
    let nverts = reader.read_u32::<LittleEndian>()?;
    let mut vertices = Vec::with_capacity(nverts as usize);
    for _ in 0..nverts {
        vertices.push(read_vec3(reader)?);
    }

    let liquid = if (liquid_flags & 1) != 0 {
        read_chunk(reader, b"LIQU")?;
        let _block_size = reader.read_i32::<LittleEndian>()?;
        let xverts = reader.read_i32::<LittleEndian>()?;
        let yverts = reader.read_i32::<LittleEndian>()?;
        let xtiles = reader.read_i32::<LittleEndian>()?;
        let ytiles = reader.read_i32::<LittleEndian>()?;
        let pos_x = reader.read_f32::<LittleEndian>()?;
        let pos_y = reader.read_f32::<LittleEndian>()?;
        let pos_z = reader.read_f32::<LittleEndian>()?;
        let liquid_type = reader.read_i16::<LittleEndian>()? as i32;
        let _pad = reader.read_u16::<LittleEndian>()?;

        let height_count = (xverts * yverts).max(0) as usize;
        let mut heights = Vec::with_capacity(height_count);
        for _ in 0..height_count {
            heights.push(reader.read_f32::<LittleEndian>()?);
        }

        let flag_count = (xtiles * ytiles).max(0) as usize;
        let mut flags = vec![0u8; flag_count];
        reader.read_exact(&mut flags)?;

        Some(WmoLiquid {
            tiles_x: xtiles.max(0) as u32,
            tiles_y: ytiles.max(0) as u32,
            corner: Vec3::new(pos_x, pos_y, pos_z),
            liquid_type: liquid_type.max(0) as u32,
            heights,
            flags,
        })
    } else {
        None
    };

    Ok(RawGroup {
        mogp_flags,
        group_wmo_id,
        bounds,
        liquid_flags,
        triangles,
        vertices,
        liquid,
    })
}

fn calculate_transformed_bound(raw_dir: &Path, spawn: &mut ModelSpawn) -> anyhow::Result<()> {
    let model = read_raw_model(&raw_dir.join(&spawn.name))?;
    if model.groups.is_empty() {
        anyhow::bail!("Model '{}' has no geometry", spawn.name);
    }

    let rotation = matrix_from_euler_zyx(
        deg_to_rad(spawn.rot.y),
        deg_to_rad(spawn.rot.x),
        deg_to_rad(spawn.rot.z),
    );

    let mut bound: Option<AaBox> = None;
    for group in &model.groups {
        for v in &group.vertices {
            let transformed = mat3_mul_vec3(rotation, v.scale(spawn.scale));
            bound = Some(match bound {
                Some(mut current) => {
                    current.merge(transformed);
                    current
                }
                None => AaBox::from_point(transformed),
            });
        }
    }

    let Some(bound) = bound else {
        anyhow::bail!("Model '{}' has no geometry", spawn.name);
    };

    spawn.bound = Some(bound.add(spawn.pos));
    spawn.flags |= MOD_HAS_BOUND;
    Ok(())
}

fn compute_model_bounds(model: &RawModel) -> Option<AaBox> {
    let mut bound: Option<AaBox> = None;
    for group in &model.groups {
        for v in &group.vertices {
            bound = Some(match bound {
                Some(mut current) => {
                    current.merge(*v);
                    current
                }
                None => AaBox::from_point(*v),
            });
        }
    }
    bound
}

fn build_map_bih(map_spawns: &[&ModelSpawn]) -> (Bih, HashMap<u32, u32>) {
    let mut node_index = HashMap::new();
    let mut prim_bounds = Vec::with_capacity(map_spawns.len());
    for (idx, spawn) in map_spawns.iter().enumerate() {
        prim_bounds.push(spawn.bound.unwrap_or_default());
        node_index.insert(spawn.id, idx as u32);
    }

    let bih = Bih::build(&prim_bounds, 3);
    (bih, node_index)
}

fn build_group_bih(groups: &[RawGroup]) -> Bih {
    let prim_bounds: Vec<AaBox> = groups.iter().map(|g| g.bounds).collect();
    Bih::build(&prim_bounds, 1)
}

fn build_mesh_bih(group: &RawGroup) -> Bih {
    let prim_bounds: Vec<AaBox> = group
        .triangles
        .iter()
        .map(|tri| {
            let mut bb = AaBox::default();
            let mut initialized = false;
            for idx in [tri.idx0, tri.idx1, tri.idx2] {
                if let Some(v) = group.vertices.get(idx as usize) {
                    if !initialized {
                        bb = AaBox::from_point(*v);
                        initialized = true;
                    } else {
                        bb.merge(*v);
                    }
                }
            }
            bb
        })
        .collect();
    Bih::build(&prim_bounds, 3)
}

const MAX_STACK_SIZE: usize = 64;

fn float_to_raw_int_bits(f: f32) -> u32 {
    f.to_bits()
}

#[derive(Clone, Debug)]
struct Bih {
    bounds: AaBox,
    tree: Vec<u32>,
    objects: Vec<u32>,
}

impl Bih {
    fn new_empty() -> Self {
        let mut tree = Vec::with_capacity(3);
        tree.push(3u32 << 30); // dummy leaf
        tree.push(0);
        tree.push(0);
        Self {
            bounds: AaBox::default(),
            tree,
            objects: Vec::new(),
        }
    }

    fn build(prim_bounds: &[AaBox], leaf_size: u32) -> Self {
        let num_prims = prim_bounds.len();
        if num_prims == 0 {
            return Self::new_empty();
        }

        let mut bounds = prim_bounds[0];
        for pb in &prim_bounds[1..] {
            bounds.merge(pb.min);
            bounds.merge(pb.max);
        }

        let mut indices: Vec<u32> = (0..num_prims as u32).collect();

        let mut temp_tree = Vec::new();
        // create space for the first node
        temp_tree.push(3u32 << 30); // dummy leaf
        temp_tree.push(0);
        temp_tree.push(0);

        let grid_box = [bounds.min, bounds.max];
        let node_box = grid_box;

        Self::subdivide(
            0,
            num_prims as i32 - 1,
            &mut temp_tree,
            &mut indices,
            prim_bounds,
            leaf_size as i32,
            grid_box,
            node_box,
            0,
            1,
        );

        let objects: Vec<u32> = (0..num_prims as u32)
            .map(|i| indices[i as usize])
            .collect();

        Self {
            bounds,
            tree: temp_tree,
            objects,
        }
    }

    fn create_node(temp_tree: &mut [u32], node_index: usize, left: u32, right: u32) {
        temp_tree[node_index] = (3u32 << 30) | left;
        temp_tree[node_index + 1] = right - left + 1;
    }

    #[allow(clippy::too_many_arguments)]
    fn subdivide(
        left: i32,
        mut right: i32,
        temp_tree: &mut Vec<u32>,
        indices: &mut [u32],
        prim_bound: &[AaBox],
        max_prims: i32,
        mut grid_box: [Vec3; 2], // [lo, hi]
        mut node_box: [Vec3; 2], // [lo, hi]
        mut node_index: usize,
        depth: usize,
    ) {
        if (right - left + 1) <= max_prims || depth >= MAX_STACK_SIZE {
            Self::create_node(temp_tree, node_index, left as u32, right as u32);
            return;
        }

        let mut axis: i32 = -1;
        let right_orig = right;
        let mut clip_l: f32;
        let mut clip_r: f32;
        let mut prev_clip: f32 = f32::NAN;
        let mut split: f32 = f32::NAN;
        let mut was_left = true;

        loop {
            let prev_axis = axis;
            let prev_split = split;

            let d = Vec3::new(
                grid_box[1].x - grid_box[0].x,
                grid_box[1].y - grid_box[0].y,
                grid_box[1].z - grid_box[0].z,
            );

            // find longest axis
            axis = if d.x >= d.y && d.x >= d.z {
                0
            } else if d.y >= d.z {
                1
            } else {
                2
            };

            let lo_axis = match axis { 0 => grid_box[0].x, 1 => grid_box[0].y, _ => grid_box[0].z };
            let hi_axis = match axis { 0 => grid_box[1].x, 1 => grid_box[1].y, _ => grid_box[1].z };
            split = 0.5 * (lo_axis + hi_axis);

            clip_l = f32::NEG_INFINITY;
            clip_r = f32::INFINITY;

            let mut node_l = f32::INFINITY;
            let mut node_r = f32::NEG_INFINITY;

            let mut i = left;
            while i <= right {
                let obj = indices[i as usize] as usize;
                let minb = match axis { 0 => prim_bound[obj].min.x, 1 => prim_bound[obj].min.y, _ => prim_bound[obj].min.z };
                let maxb = match axis { 0 => prim_bound[obj].max.x, 1 => prim_bound[obj].max.y, _ => prim_bound[obj].max.z };
                let center = (minb + maxb) * 0.5;
                if center <= split {
                    i += 1;
                    if clip_l < maxb {
                        clip_l = maxb;
                    }
                } else {
                    indices.swap(i as usize, right as usize);
                    right -= 1;
                    if clip_r > minb {
                        clip_r = minb;
                    }
                }
                node_l = node_l.min(minb);
                node_r = node_r.max(maxb);
            }

            // check for empty space
            let node_box_lo = match axis { 0 => node_box[0].x, 1 => node_box[0].y, _ => node_box[0].z };
            let node_box_hi = match axis { 0 => node_box[1].x, 1 => node_box[1].y, _ => node_box[1].z };
            if node_l > node_box_lo && node_r < node_box_hi {
                let node_box_w = node_box_hi - node_box_lo;
                let node_new_w = node_r - node_l;
                if 1.3 * node_new_w < node_box_w {
                    // BVH2 node
                    let next_index = temp_tree.len();
                    temp_tree.push(0);
                    temp_tree.push(0);
                    temp_tree.push(0);
                    temp_tree[node_index] = ((axis as u32) << 30) | (1u32 << 29) | next_index as u32;
                    temp_tree[node_index + 1] = float_to_raw_int_bits(node_l);
                    temp_tree[node_index + 2] = float_to_raw_int_bits(node_r);
                    match axis { 0 => node_box[0].x = node_l, 1 => node_box[0].y = node_l, _ => node_box[0].z = node_l };
                    match axis { 0 => node_box[1].x = node_r, 1 => node_box[1].y = node_r, _ => node_box[1].z = node_r };
                    right = right_orig;
                    Self::subdivide(left, right, temp_tree, indices, prim_bound, max_prims, grid_box, node_box, next_index, depth + 1);
                    return;
                }
            }

            // ensure we are making progress
            if right == right_orig {
                // all left
                if prev_axis == axis && !prev_split.is_nan() && (prev_split - split).abs() < 1e-6 {
                    Self::create_node(temp_tree, node_index, left as u32, right as u32);
                    return;
                }
                if clip_l <= split {
                    match axis { 0 => grid_box[1].x = split, 1 => grid_box[1].y = split, _ => grid_box[1].z = split };
                    prev_clip = clip_l;
                    was_left = true;
                    continue;
                }
                match axis { 0 => grid_box[1].x = split, 1 => grid_box[1].y = split, _ => grid_box[1].z = split };
                prev_clip = f32::NAN;
            } else if left > right {
                // all right
                right = right_orig;
                if prev_axis == axis && !prev_split.is_nan() && (prev_split - split).abs() < 1e-6 {
                    Self::create_node(temp_tree, node_index, left as u32, right as u32);
                    return;
                }
                if clip_r >= split {
                    match axis { 0 => grid_box[0].x = split, 1 => grid_box[0].y = split, _ => grid_box[0].z = split };
                    prev_clip = clip_r;
                    was_left = false;
                    continue;
                }
                match axis { 0 => grid_box[0].x = split, 1 => grid_box[0].y = split, _ => grid_box[0].z = split };
                prev_clip = f32::NAN;
            } else {
                // actual split
                if prev_axis != -1 && !prev_clip.is_nan() {
                    let next_index = temp_tree.len();
                    temp_tree.push(0);
                    temp_tree.push(0);
                    temp_tree.push(0);
                    if was_left {
                        temp_tree[node_index] = ((prev_axis as u32) << 30) | next_index as u32;
                        temp_tree[node_index + 1] = float_to_raw_int_bits(prev_clip);
                        temp_tree[node_index + 2] = float_to_raw_int_bits(f32::INFINITY);
                    } else {
                        temp_tree[node_index] = ((prev_axis as u32) << 30) | (next_index as u32).wrapping_sub(3);
                        temp_tree[node_index + 1] = float_to_raw_int_bits(f32::NEG_INFINITY);
                        temp_tree[node_index + 2] = float_to_raw_int_bits(prev_clip);
                    }
                    node_index = next_index;
                }
                break;
            }
        }

        // compute index of child nodes
        let mut next_index = temp_tree.len();
        let nl = right - left + 1;
        let nr = right_orig - (right + 1) + 1;
        if nl > 0 {
            temp_tree.push(0);
            temp_tree.push(0);
            temp_tree.push(0);
        } else {
            next_index -= 3;
        }
        if nr > 0 {
            temp_tree.push(0);
            temp_tree.push(0);
            temp_tree.push(0);
        }

        temp_tree[node_index] = ((axis as u32) << 30) | next_index as u32;
        temp_tree[node_index + 1] = float_to_raw_int_bits(clip_l);
        temp_tree[node_index + 2] = float_to_raw_int_bits(clip_r);

        let mut grid_box_l = grid_box;
        let mut grid_box_r = grid_box;
        let mut node_box_l = node_box;
        let mut node_box_r = node_box;

        match axis {
            0 => { grid_box_l[1].x = split; grid_box_r[0].x = split; node_box_l[1].x = clip_l; node_box_r[0].x = clip_r; }
            1 => { grid_box_l[1].y = split; grid_box_r[0].y = split; node_box_l[1].y = clip_l; node_box_r[0].y = clip_r; }
            _ => { grid_box_l[1].z = split; grid_box_r[0].z = split; node_box_l[1].z = clip_l; node_box_r[0].z = clip_r; }
        }

        if nl > 0 {
            Self::subdivide(left, right, temp_tree, indices, prim_bound, max_prims, grid_box_l, node_box_l, next_index, depth + 1);
        }
        if nr > 0 {
            Self::subdivide(right + 1, right_orig, temp_tree, indices, prim_bound, max_prims, grid_box_r, node_box_r, next_index + 3, depth + 1);
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        write_vec3(writer, self.bounds.min)?;
        write_vec3(writer, self.bounds.max)?;
        writer.write_u32::<LittleEndian>(self.tree.len() as u32)?;
        for value in &self.tree {
            writer.write_u32::<LittleEndian>(*value)?;
        }
        writer.write_u32::<LittleEndian>(self.objects.len() as u32)?;
        for value in &self.objects {
            writer.write_u32::<LittleEndian>(*value)?;
        }
        Ok(())
    }
}

fn read_chunk<R: Read>(reader: &mut R, expected: &[u8; 4]) -> anyhow::Result<()> {
    let mut chunk = [0u8; 4];
    reader.read_exact(&mut chunk)?;
    if &chunk != expected {
        anyhow::bail!(
            "Chunk mismatch: expected {:?}, got {:?}",
            expected,
            chunk
        );
    }
    Ok(())
}

fn read_vec3<R: Read>(reader: &mut R) -> anyhow::Result<Vec3> {
    let x = reader.read_f32::<LittleEndian>()?;
    let y = reader.read_f32::<LittleEndian>()?;
    let z = reader.read_f32::<LittleEndian>()?;
    Ok(Vec3::new(x, y, z))
}

fn write_vec3<W: Write>(writer: &mut W, v: Vec3) -> anyhow::Result<()> {
    writer.write_f32::<LittleEndian>(v.x)?;
    writer.write_f32::<LittleEndian>(v.y)?;
    writer.write_f32::<LittleEndian>(v.z)?;
    Ok(())
}

fn pack_tile_id(tile_x: u32, tile_y: u32) -> u32 {
    (tile_x << 16) | tile_y
}

fn unpack_tile_id(tile_id: u32) -> (u32, u32) {
    let tile_x = tile_id >> 16;
    let tile_y = tile_id & 0xFF;
    (tile_x, tile_y)
}

fn deg_to_rad(value: f32) -> f32 {
    value * std::f32::consts::PI / 180.0
}

fn matrix_from_euler_zyx(z: f32, y: f32, x: f32) -> [[f32; 3]; 3] {
    let (sz, cz) = z.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sx, cx) = x.sin_cos();

    [
        [cy * cz, cz * sx * sy - cx * sz, cx * cz * sy + sx * sz],
        [cy * sz, cx * cz + sx * sy * sz, -cz * sx + cx * sy * sz],
        [-sy, cy * sx, cx * cy],
    ]
}

fn mat3_mul_vec3(m: [[f32; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}
