// recast_wrapper.h - C wrapper for Recast/Detour C++ APIs
// Provides extern "C" functions callable from Rust FFI.

#ifndef RECAST_WRAPPER_H
#define RECAST_WRAPPER_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// ============================================================================
// Opaque handles for C++ objects
// ============================================================================
typedef void* rc_context_t;
typedef void* rc_heightfield_t;
typedef void* rc_compact_heightfield_t;
typedef void* rc_contour_set_t;
typedef void* rc_poly_mesh_t;
typedef void* rc_poly_mesh_detail_t;
typedef void* dt_nav_mesh_t;

// ============================================================================
// rcConfig mirror
// ============================================================================
typedef struct {
    int width;
    int height;
    int tile_size;
    int border_size;
    float cs;
    float ch;
    float bmin[3];
    float bmax[3];
    float walkable_slope_angle;
    int walkable_height;
    int walkable_climb;
    int walkable_radius;
    int max_edge_len;
    float max_simplification_error;
    int min_region_area;
    int merge_region_area;
    int max_verts_per_poly;
    float detail_sample_dist;
    float detail_sample_max_error;
    float liquid_flag_merge_threshold;
} rc_config_t;

// ============================================================================
// dtNavMeshCreateParams mirror
// ============================================================================
typedef struct {
    const uint16_t* verts;
    int vert_count;
    const uint16_t* polys;
    const uint16_t* poly_flags;
    const uint8_t* poly_areas;
    int poly_count;
    int nvp;

    const uint32_t* detail_meshes;
    const float* detail_verts;
    int detail_verts_count;
    const uint8_t* detail_tris;
    int detail_tri_count;

    const float* off_mesh_con_verts;
    const float* off_mesh_con_rad;
    const uint16_t* off_mesh_con_flags;
    const uint8_t* off_mesh_con_areas;
    const uint8_t* off_mesh_con_dir;
    const uint32_t* off_mesh_con_user_id;
    int off_mesh_con_count;

    uint32_t user_id;
    int tile_x;
    int tile_y;
    int tile_layer;
    float bmin[3];
    float bmax[3];

    float walkable_height;
    float walkable_radius;
    float walkable_climb;
    float cs;
    float ch;
    bool build_bv_tree;
} dt_nav_mesh_create_params_t;

// ============================================================================
// dtNavMeshParams mirror
// ============================================================================
typedef struct {
    float orig[3];
    float tile_width;
    float tile_height;
    int max_tiles;
    int max_polys;
} dt_nav_mesh_params_t;

// ============================================================================
// Poly mesh accessors (read fields from the opaque rcPolyMesh)
// ============================================================================
typedef struct {
    uint16_t* verts;
    uint16_t* polys;
    uint16_t* regs;
    uint16_t* flags;
    uint8_t*  areas;
    int nverts;
    int npolys;
    int maxpolys;
    int nvp;
    float bmin[3];
    float bmax[3];
    float cs;
    float ch;
    int border_size;
    float max_edge_error;
} rc_poly_mesh_data_t;

// ============================================================================
// Detail mesh accessors
// ============================================================================
typedef struct {
    uint32_t* meshes;
    float*    verts;
    uint8_t*  tris;
    int nmeshes;
    int nverts;
    int ntris;
} rc_poly_mesh_detail_data_t;

// ============================================================================
// rcContext
// ============================================================================
rc_context_t rc_alloc_context(void);
void rc_free_context(rc_context_t ctx);

// ============================================================================
// rcHeightfield
// ============================================================================
rc_heightfield_t rc_alloc_heightfield(void);
void rc_free_heightfield(rc_heightfield_t hf);
bool rc_create_heightfield(
    rc_context_t ctx, rc_heightfield_t hf,
    int width, int height,
    const float* bmin, const float* bmax,
    float cs, float ch
);

// ============================================================================
// Triangle marking & rasterization
// ============================================================================
void rc_mark_walkable_triangles(
    rc_context_t ctx, float walkable_slope_angle,
    const float* verts, int nv,
    const int* tris, int nt,
    uint8_t* areas
);

void rc_clear_unwalkable_triangles(
    rc_context_t ctx, float walkable_slope_angle,
    const float* verts, int nv,
    const int* tris, int nt,
    uint8_t* areas
);

bool rc_rasterize_triangles(
    rc_context_t ctx,
    const float* verts, int nv,
    const int* tris, const uint8_t* areas, int nt,
    rc_heightfield_t solid, int flag_merge_thr
);

// ============================================================================
// Filters
// ============================================================================
void rc_filter_low_hanging_walkable_obstacles(
    rc_context_t ctx, int walkable_climb, rc_heightfield_t hf
);
void rc_filter_ledge_spans(
    rc_context_t ctx, int walkable_height, int walkable_climb, rc_heightfield_t hf
);
void rc_filter_walkable_low_height_spans(
    rc_context_t ctx, int walkable_height, rc_heightfield_t hf
);

// ============================================================================
// Compact heightfield
// ============================================================================
rc_compact_heightfield_t rc_alloc_compact_heightfield(void);
void rc_free_compact_heightfield(rc_compact_heightfield_t chf);
bool rc_build_compact_heightfield(
    rc_context_t ctx, int walkable_height, int walkable_climb,
    rc_heightfield_t hf, rc_compact_heightfield_t chf
);

// ============================================================================
// Area processing
// ============================================================================
bool rc_erode_walkable_area(rc_context_t ctx, int radius, rc_compact_heightfield_t chf);
bool rc_median_filter_walkable_area(rc_context_t ctx, rc_compact_heightfield_t chf);

// ============================================================================
// Distance field & regions
// ============================================================================
bool rc_build_distance_field(rc_context_t ctx, rc_compact_heightfield_t chf);
bool rc_build_regions(
    rc_context_t ctx, rc_compact_heightfield_t chf,
    int border_size, int min_region_area, int merge_region_area
);

// ============================================================================
// Contours
// ============================================================================
rc_contour_set_t rc_alloc_contour_set(void);
void rc_free_contour_set(rc_contour_set_t cset);
bool rc_build_contours(
    rc_context_t ctx, rc_compact_heightfield_t chf,
    float max_error, int max_edge_len,
    rc_contour_set_t cset
);

// ============================================================================
// Poly mesh
// ============================================================================
rc_poly_mesh_t rc_alloc_poly_mesh(void);
void rc_free_poly_mesh(rc_poly_mesh_t mesh);
bool rc_build_poly_mesh(
    rc_context_t ctx, rc_contour_set_t cset,
    int nvp, rc_poly_mesh_t mesh
);
bool rc_merge_poly_meshes(
    rc_context_t ctx,
    rc_poly_mesh_t* meshes, int nmeshes,
    rc_poly_mesh_t mesh
);
void rc_get_poly_mesh_data(rc_poly_mesh_t mesh, rc_poly_mesh_data_t* out);

// ============================================================================
// Detail mesh
// ============================================================================
rc_poly_mesh_detail_t rc_alloc_poly_mesh_detail(void);
void rc_free_poly_mesh_detail(rc_poly_mesh_detail_t mesh);
bool rc_build_poly_mesh_detail(
    rc_context_t ctx, rc_poly_mesh_t mesh, rc_compact_heightfield_t chf,
    float sample_dist, float sample_max_error,
    rc_poly_mesh_detail_t dmesh
);
bool rc_merge_poly_mesh_details(
    rc_context_t ctx,
    rc_poly_mesh_detail_t* meshes, int nmeshes,
    rc_poly_mesh_detail_t mesh
);
void rc_get_poly_mesh_detail_data(rc_poly_mesh_detail_t mesh, rc_poly_mesh_detail_data_t* out);

// ============================================================================
// Detour NavMesh creation
// ============================================================================
bool dt_create_nav_mesh_data(
    dt_nav_mesh_create_params_t* params,
    uint8_t** out_data, int* out_data_size
);

void dt_free(void* ptr);

// ============================================================================
// Detour NavMesh
// ============================================================================
dt_nav_mesh_t dt_alloc_nav_mesh(void);
void dt_free_nav_mesh(dt_nav_mesh_t navmesh);
uint32_t dt_nav_mesh_init(dt_nav_mesh_t navmesh, const dt_nav_mesh_params_t* params);
uint32_t dt_nav_mesh_add_tile(
    dt_nav_mesh_t navmesh,
    uint8_t* data, int data_size,
    int flags, uint32_t last_ref, uint32_t* result
);

// DT_TILE_FREE_DATA constant
int dt_tile_free_data_flag(void);

// DT_NAVMESH_VERSION constant
int dt_navmesh_version(void);

#ifdef __cplusplus
}
#endif

#endif // RECAST_WRAPPER_H
