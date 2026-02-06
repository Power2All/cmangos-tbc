// extractors - CMaNGOS TBC extractor tools (Rust scaffold)
// Consolidated entrypoint for:
// - Map/DBC/Camera extractor (contrib/extractor/System.cpp)
// - VMap extractor (contrib/vmap_extractor/vmapextract/vmapexport.cpp)
// - VMap assembler (contrib/vmap_assembler/vmap_assembler.cpp)
// - MoveMapGen (contrib/mmap/src/generator.cpp)

use clap::{Args, Parser, Subcommand};

mod dbc;
mod map_dbc;
#[allow(dead_code, unused_variables)]
mod movemap_gen;
mod mpq;
#[cfg(feature = "recast")]
#[allow(dead_code)]
mod recast_ffi;
#[allow(dead_code, unused_variables)]
mod vmap_assemble;
#[allow(dead_code, unused_variables)]
mod vmap_extract;

use mangos_shared::log::{initialize_logging, map_log_level};

/// Extractor selection bitmask
const EXTRACT_MAP: u8 = 1;
const EXTRACT_DBC: u8 = 2;
const EXTRACT_CAMERA: u8 = 4;
const DEFAULT_EXTRACT_MASK: u8 = EXTRACT_MAP | EXTRACT_DBC | EXTRACT_CAMERA;

#[derive(Parser, Debug)]
#[command(name = "extractors")]
#[command(about = "CMaNGOS TBC Extractor Tools (Rust scaffold)")]
#[command(version)]
struct Cli {
    /// Console log level override (0=Minimum, 1=Error, 2=Detail, 3=Full/Debug, 4=Trace)
    #[arg(short, long, value_name = "LEVEL")]
    log_level: Option<i32>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Map/DBC/Camera extractor (C++: MapDbcExtractor)
    MapDbc(MapDbcArgs),
    /// VMap extractor (C++: VMapExtractor)
    VmapExtract(VmapExtractArgs),
    /// VMap assembler (C++: VMapAssembler)
    VmapAssemble(VmapAssembleArgs),
    /// MoveMap generator (C++: MoveMapGen)
    MoveMapGen(MoveMapGenArgs),
}

#[derive(Args, Debug)]
struct MapDbcArgs {
    /// Input path (game directory)
    #[arg(short = 'i', long = "input", default_value = ".")]
    input_path: String,

    /// Output path
    #[arg(short = 'o', long = "output", default_value = ".")]
    output_path: String,

    /// Extract only MAP(1)/DBC(2)/Camera(4); default is all (7)
    #[arg(short = 'e', long = "extract", default_value_t = DEFAULT_EXTRACT_MASK)]
    extract_mask: u8,

    /// Store height as integer values: 1 = enabled, 0 = disabled
    #[arg(short = 'f', long = "float-to-int", default_value_t = 1)]
    float_to_int: u8,

    /// Clamp heights below this minimum value
    #[arg(long = "min-height", default_value_t = -500.0)]
    min_height: f32,

    /// Disable clamping of minimum height
    #[arg(long = "disable-min-height-limit", default_value_t = false)]
    disable_min_height_limit: bool,

    /// Number of threads to use
    #[arg(long = "threads")]
    threads: Option<usize>,
}

#[derive(Args, Debug)]
struct VmapExtractArgs {
    /// Path to the game data directory (Data/)
    #[arg(short = 'd', long = "data", default_value = ".")]
    data_path: String,

    /// Output directory
    #[arg(short = 'o', long = "output", default_value = ".")]
    output_path: String,

    /// Large size (more precise vector data, larger output)
    #[arg(short = 'l', long = "large", conflicts_with = "small")]
    large: bool,

    /// Small size (default, smaller output)
    #[arg(short = 's', long = "small")]
    small: bool,

    /// Number of threads to use
    #[arg(long = "threads")]
    threads: Option<usize>,
}

#[derive(Args, Debug)]
struct VmapAssembleArgs {
    /// Raw data directory
    raw_data_dir: String,

    /// Output vmap directory
    output_dir: String,

    /// Number of threads to use
    #[arg(long = "threads")]
    threads: Option<usize>,
}

#[derive(Clone, Debug)]
struct Tile {
    x: i32,
    y: i32,
}

fn parse_tile(input: &str) -> Result<Tile, String> {
    let mut parts = input.split(',');
    let x = parts
        .next()
        .ok_or_else(|| "Missing tile X".to_string())?
        .parse::<i32>()
        .map_err(|_| "Invalid tile X".to_string())?;
    let y = parts
        .next()
        .ok_or_else(|| "Missing tile Y".to_string())?
        .parse::<i32>()
        .map_err(|_| "Invalid tile Y".to_string())?;
    Ok(Tile { x, y })
}

#[derive(Args, Debug)]
struct MoveMapGenArgs {
    /// Map IDs to build (space-separated)
    map_ids: Vec<u32>,

    /// Build the specified tile (format: X,Y)
    #[arg(long = "tile", value_parser = parse_tile)]
    tile: Option<Tile>,

    /// Skip liquid data
    #[arg(long = "skipLiquid")]
    skip_liquid: bool,

    /// Skip continents
    #[arg(long = "skipContinents")]
    skip_continents: bool,

    /// Skip junk maps
    #[arg(long = "skipJunkMaps")]
    skip_junk_maps: bool,

    /// Skip battlegrounds
    #[arg(long = "skipBattlegrounds")]
    skip_battlegrounds: bool,

    /// Create debug output for RecastDemo
    #[arg(long = "debug")]
    debug_output: bool,

    /// Script-friendly mode (no prompts)
    #[arg(long = "silent")]
    silent: bool,

    /// Build gameobject models for transports
    #[arg(long = "buildGameObjects")]
    build_game_objects: bool,

    /// Off-mesh connection input file
    #[arg(long = "offMeshInput", default_value = "offmesh.txt")]
    off_mesh_input: String,

    /// JSON configuration file path
    #[arg(long = "configInputPath", default_value = "config.json")]
    config_input: String,

    /// Number of threads to use
    #[arg(long = "threads")]
    threads: Option<usize>,

    /// Base work directory (fallback for maps/vmaps/mmaps if not specified individually)
    #[arg(long = "workdir", default_value = "./")]
    workdir: String,

    /// Custom path to maps directory (overrides workdir/maps)
    #[arg(long = "mapsDir")]
    maps_dir: Option<String>,

    /// Custom path to vmaps directory (overrides workdir/vmaps)
    #[arg(long = "vmapsDir")]
    vmaps_dir: Option<String>,

    /// Custom path to mmaps output directory (overrides workdir/mmaps)
    #[arg(long = "mmapsDir")]
    mmaps_dir: Option<String>,
}

fn init_logging(log_level: Option<i32>) {
    let console_level = map_log_level(log_level.unwrap_or(2));
    initialize_logging(None, console_level, None);
}

#[allow(dead_code)]
fn ensure_dir(path: &str) -> anyhow::Result<()> {
    let dir = std::path::Path::new(path);
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn resolve_threads(threads: Option<usize>) -> usize {
    threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

fn run_map_dbc(args: MapDbcArgs) -> anyhow::Result<()> {
    let threads = resolve_threads(args.threads);
    tracing::info!("MapDbc: threads={}", threads);
    map_dbc::run_map_dbc(args, threads)
}

fn run_vmap_extract(args: VmapExtractArgs) -> anyhow::Result<()> {
    let threads = resolve_threads(args.threads);
    tracing::info!("VmapExtract: threads={}", threads);
    vmap_extract::run_vmap_extract(args, threads)
}

fn run_vmap_assemble(args: VmapAssembleArgs) -> anyhow::Result<()> {
    let threads = resolve_threads(args.threads);
    tracing::info!("VmapAssemble: threads={}", threads);
    vmap_assemble::run_vmap_assemble(args, threads)
}

fn run_movemap_gen(args: MoveMapGenArgs) -> anyhow::Result<()> {
    let tile_info = args.tile.as_ref().map(|tile| format!("{},{}", tile.x, tile.y));
    let threads = args
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));

    tracing::info!(
        "MoveMapGen: workdir='{}' tile={:?} maps={:?} threads={} debug={} silent={} build_game_objects={}",
        args.workdir,
        tile_info,
        args.map_ids,
        threads,
        args.debug_output,
        args.silent,
        args.build_game_objects
    );

    movemap_gen::run_movemap_gen(&args)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    init_logging(cli.log_level);

    match cli.command {
        Command::MapDbc(args) => run_map_dbc(args),
        Command::VmapExtract(args) => run_vmap_extract(args),
        Command::VmapAssemble(args) => run_vmap_assemble(args),
        Command::MoveMapGen(args) => run_movemap_gen(args),
    }
}
