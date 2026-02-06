// recast_wrapper.cpp - C wrapper implementation for Recast/Detour C++ APIs
// Bridges C++ classes/methods to extern "C" functions for Rust FFI.

#include "Recast.h"
#include "DetourAlloc.h"
#include "DetourCommon.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshBuilder.h"
#include "recast_wrapper.h"

#include <cstring>

// ============================================================================
// rcContext wrapper (minimal - no logging/timing)
// ============================================================================

extern "C" rc_context_t rc_alloc_context(void)
{
    return static_cast<rc_context_t>(new rcContext(false));
}

extern "C" void rc_free_context(rc_context_t ctx)
{
    delete static_cast<rcContext*>(ctx);
}

// ============================================================================
// rcHeightfield
// ============================================================================

extern "C" rc_heightfield_t rc_alloc_heightfield(void)
{
    return static_cast<rc_heightfield_t>(rcAllocHeightfield());
}

extern "C" void rc_free_heightfield(rc_heightfield_t hf)
{
    rcFreeHeightField(static_cast<rcHeightfield*>(hf));
}

extern "C" bool rc_create_heightfield(
    rc_context_t ctx, rc_heightfield_t hf,
    int width, int height,
    const float* bmin, const float* bmax,
    float cs, float ch)
{
    return rcCreateHeightfield(
        static_cast<rcContext*>(ctx),
        *static_cast<rcHeightfield*>(hf),
        width, height,
        bmin, bmax,
        cs, ch
    );
}

// ============================================================================
// Triangle marking & rasterization
// ============================================================================

extern "C" void rc_mark_walkable_triangles(
    rc_context_t ctx, float walkable_slope_angle,
    const float* verts, int nv,
    const int* tris, int nt,
    uint8_t* areas)
{
    rcMarkWalkableTriangles(
        static_cast<rcContext*>(ctx),
        walkable_slope_angle,
        verts, nv,
        tris, nt,
        areas
    );
}

extern "C" void rc_clear_unwalkable_triangles(
    rc_context_t ctx, float walkable_slope_angle,
    const float* verts, int nv,
    const int* tris, int nt,
    uint8_t* areas)
{
    rcClearUnwalkableTriangles(
        static_cast<rcContext*>(ctx),
        walkable_slope_angle,
        verts, nv,
        tris, nt,
        areas
    );
}

extern "C" bool rc_rasterize_triangles(
    rc_context_t ctx,
    const float* verts, int nv,
    const int* tris, const uint8_t* areas, int nt,
    rc_heightfield_t solid, int flag_merge_thr)
{
    return rcRasterizeTriangles(
        static_cast<rcContext*>(ctx),
        verts, nv,
        tris, areas, nt,
        *static_cast<rcHeightfield*>(solid),
        flag_merge_thr
    );
}

// ============================================================================
// Filters
// ============================================================================

extern "C" void rc_filter_low_hanging_walkable_obstacles(
    rc_context_t ctx, int walkable_climb, rc_heightfield_t hf)
{
    rcFilterLowHangingWalkableObstacles(
        static_cast<rcContext*>(ctx),
        walkable_climb,
        *static_cast<rcHeightfield*>(hf)
    );
}

extern "C" void rc_filter_ledge_spans(
    rc_context_t ctx, int walkable_height, int walkable_climb, rc_heightfield_t hf)
{
    rcFilterLedgeSpans(
        static_cast<rcContext*>(ctx),
        walkable_height, walkable_climb,
        *static_cast<rcHeightfield*>(hf)
    );
}

extern "C" void rc_filter_walkable_low_height_spans(
    rc_context_t ctx, int walkable_height, rc_heightfield_t hf)
{
    rcFilterWalkableLowHeightSpans(
        static_cast<rcContext*>(ctx),
        walkable_height,
        *static_cast<rcHeightfield*>(hf)
    );
}

// ============================================================================
// Compact heightfield
// ============================================================================

extern "C" rc_compact_heightfield_t rc_alloc_compact_heightfield(void)
{
    return static_cast<rc_compact_heightfield_t>(rcAllocCompactHeightfield());
}

extern "C" void rc_free_compact_heightfield(rc_compact_heightfield_t chf)
{
    rcFreeCompactHeightfield(static_cast<rcCompactHeightfield*>(chf));
}

extern "C" bool rc_build_compact_heightfield(
    rc_context_t ctx, int walkable_height, int walkable_climb,
    rc_heightfield_t hf, rc_compact_heightfield_t chf)
{
    return rcBuildCompactHeightfield(
        static_cast<rcContext*>(ctx),
        walkable_height, walkable_climb,
        *static_cast<rcHeightfield*>(hf),
        *static_cast<rcCompactHeightfield*>(chf)
    );
}

// ============================================================================
// Area processing
// ============================================================================

extern "C" bool rc_erode_walkable_area(
    rc_context_t ctx, int radius, rc_compact_heightfield_t chf)
{
    return rcErodeWalkableArea(
        static_cast<rcContext*>(ctx),
        radius,
        *static_cast<rcCompactHeightfield*>(chf)
    );
}

extern "C" bool rc_median_filter_walkable_area(
    rc_context_t ctx, rc_compact_heightfield_t chf)
{
    return rcMedianFilterWalkableArea(
        static_cast<rcContext*>(ctx),
        *static_cast<rcCompactHeightfield*>(chf)
    );
}

// ============================================================================
// Distance field & regions
// ============================================================================

extern "C" bool rc_build_distance_field(
    rc_context_t ctx, rc_compact_heightfield_t chf)
{
    return rcBuildDistanceField(
        static_cast<rcContext*>(ctx),
        *static_cast<rcCompactHeightfield*>(chf)
    );
}

extern "C" bool rc_build_regions(
    rc_context_t ctx, rc_compact_heightfield_t chf,
    int border_size, int min_region_area, int merge_region_area)
{
    return rcBuildRegions(
        static_cast<rcContext*>(ctx),
        *static_cast<rcCompactHeightfield*>(chf),
        border_size, min_region_area, merge_region_area
    );
}

// ============================================================================
// Contours
// ============================================================================

extern "C" rc_contour_set_t rc_alloc_contour_set(void)
{
    return static_cast<rc_contour_set_t>(rcAllocContourSet());
}

extern "C" void rc_free_contour_set(rc_contour_set_t cset)
{
    rcFreeContourSet(static_cast<rcContourSet*>(cset));
}

extern "C" bool rc_build_contours(
    rc_context_t ctx, rc_compact_heightfield_t chf,
    float max_error, int max_edge_len,
    rc_contour_set_t cset)
{
    return rcBuildContours(
        static_cast<rcContext*>(ctx),
        *static_cast<rcCompactHeightfield*>(chf),
        max_error, max_edge_len,
        *static_cast<rcContourSet*>(cset),
        RC_CONTOUR_TESS_WALL_EDGES
    );
}

// ============================================================================
// Poly mesh
// ============================================================================

extern "C" rc_poly_mesh_t rc_alloc_poly_mesh(void)
{
    return static_cast<rc_poly_mesh_t>(rcAllocPolyMesh());
}

extern "C" void rc_free_poly_mesh(rc_poly_mesh_t mesh)
{
    rcFreePolyMesh(static_cast<rcPolyMesh*>(mesh));
}

extern "C" bool rc_build_poly_mesh(
    rc_context_t ctx, rc_contour_set_t cset,
    int nvp, rc_poly_mesh_t mesh)
{
    return rcBuildPolyMesh(
        static_cast<rcContext*>(ctx),
        *static_cast<rcContourSet*>(cset),
        nvp,
        *static_cast<rcPolyMesh*>(mesh)
    );
}

extern "C" bool rc_merge_poly_meshes(
    rc_context_t ctx,
    rc_poly_mesh_t* meshes, int nmeshes,
    rc_poly_mesh_t mesh)
{
    // Cast array of opaque pointers to rcPolyMesh** array
    return rcMergePolyMeshes(
        static_cast<rcContext*>(ctx),
        reinterpret_cast<rcPolyMesh**>(meshes),
        nmeshes,
        *static_cast<rcPolyMesh*>(mesh)
    );
}

extern "C" void rc_get_poly_mesh_data(rc_poly_mesh_t mesh, rc_poly_mesh_data_t* out)
{
    rcPolyMesh* pm = static_cast<rcPolyMesh*>(mesh);
    out->verts = pm->verts;
    out->polys = pm->polys;
    out->regs = pm->regs;
    out->flags = pm->flags;
    out->areas = pm->areas;
    out->nverts = pm->nverts;
    out->npolys = pm->npolys;
    out->maxpolys = pm->maxpolys;
    out->nvp = pm->nvp;
    out->bmin[0] = pm->bmin[0];
    out->bmin[1] = pm->bmin[1];
    out->bmin[2] = pm->bmin[2];
    out->bmax[0] = pm->bmax[0];
    out->bmax[1] = pm->bmax[1];
    out->bmax[2] = pm->bmax[2];
    out->cs = pm->cs;
    out->ch = pm->ch;
    out->border_size = pm->borderSize;
    out->max_edge_error = pm->maxEdgeError;
}

// ============================================================================
// Detail mesh
// ============================================================================

extern "C" rc_poly_mesh_detail_t rc_alloc_poly_mesh_detail(void)
{
    return static_cast<rc_poly_mesh_detail_t>(rcAllocPolyMeshDetail());
}

extern "C" void rc_free_poly_mesh_detail(rc_poly_mesh_detail_t mesh)
{
    rcFreePolyMeshDetail(static_cast<rcPolyMeshDetail*>(mesh));
}

extern "C" bool rc_build_poly_mesh_detail(
    rc_context_t ctx, rc_poly_mesh_t mesh, rc_compact_heightfield_t chf,
    float sample_dist, float sample_max_error,
    rc_poly_mesh_detail_t dmesh)
{
    return rcBuildPolyMeshDetail(
        static_cast<rcContext*>(ctx),
        *static_cast<rcPolyMesh*>(mesh),
        *static_cast<rcCompactHeightfield*>(chf),
        sample_dist, sample_max_error,
        *static_cast<rcPolyMeshDetail*>(dmesh)
    );
}

extern "C" bool rc_merge_poly_mesh_details(
    rc_context_t ctx,
    rc_poly_mesh_detail_t* meshes, int nmeshes,
    rc_poly_mesh_detail_t mesh)
{
    return rcMergePolyMeshDetails(
        static_cast<rcContext*>(ctx),
        reinterpret_cast<rcPolyMeshDetail**>(meshes),
        nmeshes,
        *static_cast<rcPolyMeshDetail*>(mesh)
    );
}

extern "C" void rc_get_poly_mesh_detail_data(rc_poly_mesh_detail_t mesh, rc_poly_mesh_detail_data_t* out)
{
    rcPolyMeshDetail* dm = static_cast<rcPolyMeshDetail*>(mesh);
    out->meshes = dm->meshes;
    out->verts = dm->verts;
    out->tris = dm->tris;
    out->nmeshes = dm->nmeshes;
    out->nverts = dm->nverts;
    out->ntris = dm->ntris;
}

// ============================================================================
// Detour NavMesh creation
// ============================================================================

extern "C" bool dt_create_nav_mesh_data(
    dt_nav_mesh_create_params_t* params,
    uint8_t** out_data, int* out_data_size)
{
    // Convert our C struct to the C++ dtNavMeshCreateParams
    dtNavMeshCreateParams dt_params;
    memset(&dt_params, 0, sizeof(dt_params));

    dt_params.verts = params->verts;
    dt_params.vertCount = params->vert_count;
    dt_params.polys = params->polys;
    dt_params.polyFlags = params->poly_flags;
    dt_params.polyAreas = params->poly_areas;
    dt_params.polyCount = params->poly_count;
    dt_params.nvp = params->nvp;

    dt_params.detailMeshes = params->detail_meshes;
    dt_params.detailVerts = params->detail_verts;
    dt_params.detailVertsCount = params->detail_verts_count;
    dt_params.detailTris = params->detail_tris;
    dt_params.detailTriCount = params->detail_tri_count;

    dt_params.offMeshConVerts = params->off_mesh_con_verts;
    dt_params.offMeshConRad = params->off_mesh_con_rad;
    dt_params.offMeshConFlags = params->off_mesh_con_flags;
    dt_params.offMeshConAreas = params->off_mesh_con_areas;
    dt_params.offMeshConDir = params->off_mesh_con_dir;
    dt_params.offMeshConUserID = params->off_mesh_con_user_id;
    dt_params.offMeshConCount = params->off_mesh_con_count;

    dt_params.userId = params->user_id;
    dt_params.tileX = params->tile_x;
    dt_params.tileY = params->tile_y;
    dt_params.tileLayer = params->tile_layer;
    dt_params.bmin[0] = params->bmin[0];
    dt_params.bmin[1] = params->bmin[1];
    dt_params.bmin[2] = params->bmin[2];
    dt_params.bmax[0] = params->bmax[0];
    dt_params.bmax[1] = params->bmax[1];
    dt_params.bmax[2] = params->bmax[2];

    dt_params.walkableHeight = params->walkable_height;
    dt_params.walkableRadius = params->walkable_radius;
    dt_params.walkableClimb = params->walkable_climb;
    dt_params.cs = params->cs;
    dt_params.ch = params->ch;
    dt_params.buildBvTree = params->build_bv_tree;

    unsigned char* data = nullptr;
    int data_size = 0;
    bool result = dtCreateNavMeshData(&dt_params, &data, &data_size);

    *out_data = data;
    *out_data_size = data_size;

    return result;
}

extern "C" void dt_free(void* ptr)
{
    dtFree(ptr);
}

// ============================================================================
// Detour NavMesh
// ============================================================================

extern "C" dt_nav_mesh_t dt_alloc_nav_mesh(void)
{
    return static_cast<dt_nav_mesh_t>(dtAllocNavMesh());
}

extern "C" void dt_free_nav_mesh(dt_nav_mesh_t navmesh)
{
    dtFreeNavMesh(static_cast<dtNavMesh*>(navmesh));
}

extern "C" uint32_t dt_nav_mesh_init(
    dt_nav_mesh_t navmesh, const dt_nav_mesh_params_t* params)
{
    dtNavMeshParams dt_params;
    dt_params.orig[0] = params->orig[0];
    dt_params.orig[1] = params->orig[1];
    dt_params.orig[2] = params->orig[2];
    dt_params.tileWidth = params->tile_width;
    dt_params.tileHeight = params->tile_height;
    dt_params.maxTiles = params->max_tiles;
    dt_params.maxPolys = params->max_polys;

    return static_cast<dtNavMesh*>(navmesh)->init(&dt_params);
}

extern "C" uint32_t dt_nav_mesh_add_tile(
    dt_nav_mesh_t navmesh,
    uint8_t* data, int data_size,
    int flags, uint32_t last_ref, uint32_t* result)
{
    dtTileRef tile_ref = 0;
    dtStatus status = static_cast<dtNavMesh*>(navmesh)->addTile(
        data, data_size, flags, static_cast<dtTileRef>(last_ref), &tile_ref
    );
    if (result) {
        *result = static_cast<uint32_t>(tile_ref);
    }
    return status;
}

extern "C" int dt_tile_free_data_flag(void)
{
    return DT_TILE_FREE_DATA;
}

extern "C" int dt_navmesh_version(void)
{
    return DT_NAVMESH_VERSION;
}
