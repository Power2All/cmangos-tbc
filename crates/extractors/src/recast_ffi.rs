// recast_ffi.rs - Rust FFI bindings for our Recast/Detour C wrapper
//
// These bindings match the extern "C" functions in recast_wrapper.cpp,
// which in turn wraps the C++ Recast/Detour APIs from the project's
// bundled dep/recastnavigation/ source.

#![allow(non_camel_case_types)]

use std::ffi::c_void;

/// Opaque handle for rcContext
pub type rc_context_t = *mut c_void;
/// Opaque handle for rcHeightfield
pub type rc_heightfield_t = *mut c_void;
/// Opaque handle for rcCompactHeightfield
pub type rc_compact_heightfield_t = *mut c_void;
/// Opaque handle for rcContourSet
pub type rc_contour_set_t = *mut c_void;
/// Opaque handle for rcPolyMesh
pub type rc_poly_mesh_t = *mut c_void;
/// Opaque handle for rcPolyMeshDetail
pub type rc_poly_mesh_detail_t = *mut c_void;
/// Opaque handle for dtNavMesh
pub type dt_nav_mesh_t = *mut c_void;

/// Mirror of rc_config_t in recast_wrapper.h
#[repr(C)]
#[derive(Clone, Default)]
pub struct RcConfigC {
    pub width: i32,
    pub height: i32,
    pub tile_size: i32,
    pub border_size: i32,
    pub cs: f32,
    pub ch: f32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub walkable_slope_angle: f32,
    pub walkable_height: i32,
    pub walkable_climb: i32,
    pub walkable_radius: i32,
    pub max_edge_len: i32,
    pub max_simplification_error: f32,
    pub min_region_area: i32,
    pub merge_region_area: i32,
    pub max_verts_per_poly: i32,
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
    pub liquid_flag_merge_threshold: f32,
}

/// Mirror of dt_nav_mesh_create_params_t in recast_wrapper.h
#[repr(C)]
pub struct DtNavMeshCreateParamsC {
    pub verts: *const u16,
    pub vert_count: i32,
    pub polys: *const u16,
    pub poly_flags: *const u16,
    pub poly_areas: *const u8,
    pub poly_count: i32,
    pub nvp: i32,

    pub detail_meshes: *const u32,
    pub detail_verts: *const f32,
    pub detail_verts_count: i32,
    pub detail_tris: *const u8,
    pub detail_tri_count: i32,

    pub off_mesh_con_verts: *const f32,
    pub off_mesh_con_rad: *const f32,
    pub off_mesh_con_flags: *const u16,
    pub off_mesh_con_areas: *const u8,
    pub off_mesh_con_dir: *const u8,
    pub off_mesh_con_user_id: *const u32,
    pub off_mesh_con_count: i32,

    pub user_id: u32,
    pub tile_x: i32,
    pub tile_y: i32,
    pub tile_layer: i32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],

    pub walkable_height: f32,
    pub walkable_radius: f32,
    pub walkable_climb: f32,
    pub cs: f32,
    pub ch: f32,
    pub build_bv_tree: bool,
}

/// Mirror of dt_nav_mesh_params_t in recast_wrapper.h
#[repr(C)]
pub struct DtNavMeshParamsC {
    pub orig: [f32; 3],
    pub tile_width: f32,
    pub tile_height: f32,
    pub max_tiles: i32,
    pub max_polys: i32,
}

/// Mirror of rc_poly_mesh_data_t in recast_wrapper.h
#[repr(C)]
pub struct RcPolyMeshDataC {
    pub verts: *mut u16,
    pub polys: *mut u16,
    pub regs: *mut u16,
    pub flags: *mut u16,
    pub areas: *mut u8,
    pub nverts: i32,
    pub npolys: i32,
    pub maxpolys: i32,
    pub nvp: i32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub border_size: i32,
    pub max_edge_error: f32,
}

/// Mirror of rc_poly_mesh_detail_data_t in recast_wrapper.h
#[repr(C)]
pub struct RcPolyMeshDetailDataC {
    pub meshes: *mut u32,
    pub verts: *mut f32,
    pub tris: *mut u8,
    pub nmeshes: i32,
    pub nverts: i32,
    pub ntris: i32,
}

impl Default for DtNavMeshCreateParamsC {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

unsafe impl Send for RcPolyMeshDataC {}
unsafe impl Sync for RcPolyMeshDataC {}
unsafe impl Send for RcPolyMeshDetailDataC {}
unsafe impl Sync for RcPolyMeshDetailDataC {}

unsafe extern "C" {
    // rcContext
    pub fn rc_alloc_context() -> rc_context_t;
    pub fn rc_free_context(ctx: rc_context_t);

    // rcHeightfield
    pub fn rc_alloc_heightfield() -> rc_heightfield_t;
    pub fn rc_free_heightfield(hf: rc_heightfield_t);
    pub fn rc_create_heightfield(
        ctx: rc_context_t,
        hf: rc_heightfield_t,
        width: i32,
        height: i32,
        bmin: *const f32,
        bmax: *const f32,
        cs: f32,
        ch: f32,
    ) -> bool;

    // Triangle marking & rasterization
    pub fn rc_mark_walkable_triangles(
        ctx: rc_context_t,
        walkable_slope_angle: f32,
        verts: *const f32,
        nv: i32,
        tris: *const i32,
        nt: i32,
        areas: *mut u8,
    );

    pub fn rc_clear_unwalkable_triangles(
        ctx: rc_context_t,
        walkable_slope_angle: f32,
        verts: *const f32,
        nv: i32,
        tris: *const i32,
        nt: i32,
        areas: *mut u8,
    );

    pub fn rc_rasterize_triangles(
        ctx: rc_context_t,
        verts: *const f32,
        nv: i32,
        tris: *const i32,
        areas: *const u8,
        nt: i32,
        solid: rc_heightfield_t,
        flag_merge_thr: i32,
    ) -> bool;

    // Filters
    pub fn rc_filter_low_hanging_walkable_obstacles(
        ctx: rc_context_t,
        walkable_climb: i32,
        hf: rc_heightfield_t,
    );
    pub fn rc_filter_ledge_spans(
        ctx: rc_context_t,
        walkable_height: i32,
        walkable_climb: i32,
        hf: rc_heightfield_t,
    );
    pub fn rc_filter_walkable_low_height_spans(
        ctx: rc_context_t,
        walkable_height: i32,
        hf: rc_heightfield_t,
    );

    // Compact heightfield
    pub fn rc_alloc_compact_heightfield() -> rc_compact_heightfield_t;
    pub fn rc_free_compact_heightfield(chf: rc_compact_heightfield_t);
    pub fn rc_build_compact_heightfield(
        ctx: rc_context_t,
        walkable_height: i32,
        walkable_climb: i32,
        hf: rc_heightfield_t,
        chf: rc_compact_heightfield_t,
    ) -> bool;

    // Area processing
    pub fn rc_erode_walkable_area(
        ctx: rc_context_t,
        radius: i32,
        chf: rc_compact_heightfield_t,
    ) -> bool;
    pub fn rc_median_filter_walkable_area(
        ctx: rc_context_t,
        chf: rc_compact_heightfield_t,
    ) -> bool;

    // Distance field & regions
    pub fn rc_build_distance_field(
        ctx: rc_context_t,
        chf: rc_compact_heightfield_t,
    ) -> bool;
    pub fn rc_build_regions(
        ctx: rc_context_t,
        chf: rc_compact_heightfield_t,
        border_size: i32,
        min_region_area: i32,
        merge_region_area: i32,
    ) -> bool;

    // Contours
    pub fn rc_alloc_contour_set() -> rc_contour_set_t;
    pub fn rc_free_contour_set(cset: rc_contour_set_t);
    pub fn rc_build_contours(
        ctx: rc_context_t,
        chf: rc_compact_heightfield_t,
        max_error: f32,
        max_edge_len: i32,
        cset: rc_contour_set_t,
    ) -> bool;

    // Poly mesh
    pub fn rc_alloc_poly_mesh() -> rc_poly_mesh_t;
    pub fn rc_free_poly_mesh(mesh: rc_poly_mesh_t);
    pub fn rc_build_poly_mesh(
        ctx: rc_context_t,
        cset: rc_contour_set_t,
        nvp: i32,
        mesh: rc_poly_mesh_t,
    ) -> bool;
    pub fn rc_merge_poly_meshes(
        ctx: rc_context_t,
        meshes: *mut rc_poly_mesh_t,
        nmeshes: i32,
        mesh: rc_poly_mesh_t,
    ) -> bool;
    pub fn rc_get_poly_mesh_data(mesh: rc_poly_mesh_t, out: *mut RcPolyMeshDataC);

    // Detail mesh
    pub fn rc_alloc_poly_mesh_detail() -> rc_poly_mesh_detail_t;
    pub fn rc_free_poly_mesh_detail(mesh: rc_poly_mesh_detail_t);
    pub fn rc_build_poly_mesh_detail(
        ctx: rc_context_t,
        mesh: rc_poly_mesh_t,
        chf: rc_compact_heightfield_t,
        sample_dist: f32,
        sample_max_error: f32,
        dmesh: rc_poly_mesh_detail_t,
    ) -> bool;
    pub fn rc_merge_poly_mesh_details(
        ctx: rc_context_t,
        meshes: *mut rc_poly_mesh_detail_t,
        nmeshes: i32,
        mesh: rc_poly_mesh_detail_t,
    ) -> bool;
    pub fn rc_get_poly_mesh_detail_data(
        mesh: rc_poly_mesh_detail_t,
        out: *mut RcPolyMeshDetailDataC,
    );

    // Detour NavMesh creation
    pub fn dt_create_nav_mesh_data(
        params: *mut DtNavMeshCreateParamsC,
        out_data: *mut *mut u8,
        out_data_size: *mut i32,
    ) -> bool;
    pub fn dt_free(ptr: *mut c_void);

    // Detour NavMesh
    pub fn dt_alloc_nav_mesh() -> dt_nav_mesh_t;
    pub fn dt_free_nav_mesh(navmesh: dt_nav_mesh_t);
    pub fn dt_nav_mesh_init(navmesh: dt_nav_mesh_t, params: *const DtNavMeshParamsC) -> u32;
    pub fn dt_nav_mesh_add_tile(
        navmesh: dt_nav_mesh_t,
        data: *mut u8,
        data_size: i32,
        flags: i32,
        last_ref: u32,
        result: *mut u32,
    ) -> u32;

    pub fn dt_tile_free_data_flag() -> i32;
    pub fn dt_navmesh_version() -> i32;
}
