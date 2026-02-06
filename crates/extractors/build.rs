// build.rs - Compile bundled Recast/Detour C++ source for MoveMapGen
//
// Only compiles when the "recast" feature is enabled.
// Uses thirdparty/recastnavigation/ (self-contained within RustCode/)
// to ensure binary compatibility with the C++ MoveMapGen output.

fn main() {
    #[cfg(feature = "recast")]
    build_recast_detour();
}

#[cfg(feature = "recast")]
fn build_recast_detour() {
    // Path relative to this crate's Cargo.toml (crates/extractors/)
    // Points to RustCode/thirdparty/recastnavigation/
    let recast_dir = std::path::Path::new("../../thirdparty/recastnavigation");

    // Recast source files
    let recast_src = recast_dir.join("Recast/Source");
    let recast_sources = [
        "Recast.cpp",
        "RecastAlloc.cpp",
        "RecastArea.cpp",
        "RecastAssert.cpp",
        "RecastContour.cpp",
        "RecastFilter.cpp",
        "RecastLayers.cpp",
        "RecastMesh.cpp",
        "RecastMeshDetail.cpp",
        "RecastRasterization.cpp",
        "RecastRegion.cpp",
    ];

    // Detour source files
    let detour_src = recast_dir.join("Detour/Source");
    let detour_sources = [
        "DetourAlloc.cpp",
        "DetourAssert.cpp",
        "DetourCommon.cpp",
        "DetourNavMesh.cpp",
        "DetourNavMeshBuilder.cpp",
        "DetourNavMeshQuery.cpp",
        "DetourNode.cpp",
    ];

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++14")
        .warnings(false)
        .include(recast_dir.join("Recast/Include"))
        .include(recast_dir.join("Detour/Include"));

    // Add Recast sources
    for src in &recast_sources {
        build.file(recast_src.join(src));
    }

    // Add Detour sources
    for src in &detour_sources {
        build.file(detour_src.join(src));
    }

    // Add our C++ wrapper
    build.file("recast_wrapper.cpp");

    // Include the wrapper header directory
    build.include(".");

    // Compile
    build.compile("recastdetour");

    // Re-run if any source changes
    println!("cargo:rerun-if-changed=recast_wrapper.cpp");
    println!("cargo:rerun-if-changed=recast_wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
}
