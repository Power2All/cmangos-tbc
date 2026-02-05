use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use byteorder::{LittleEndian, WriteBytesExt};
use wow_adt::chunks::mh2o::VertexDataArray;
use wow_adt::{parse_adt, ParsedAdt};
use wow_wdt::{version::WowVersion, WdtReader};

use crate::dbc::DbcFile;
use crate::mpq::{build_path, MpqManager};
use crate::MapDbcArgs;

const LANGS: [&str; 12] = [
    "enGB", "enUS", "deDE", "esES", "frFR", "koKR", "zhCN", "zhTW", "enCN", "enTW", "esMX", "ruRU",
];

const CONF_MPQ_LIST: [&str; 9] = [
    "common.MPQ",
    "common-2.MPQ",
    "lichking.MPQ",
    "expansion.MPQ",
    "patch.MPQ",
    "patch-2.MPQ",
    "patch-3.MPQ",
    "patch-4.MPQ",
    "patch-5.MPQ",
];

const EXTRACT_MAP: u8 = 1;
const EXTRACT_DBC: u8 = 2;
const EXTRACT_CAMERA: u8 = 4;

const ADT_CELLS_PER_GRID: usize = 16;
const ADT_CELL_SIZE: usize = 8;
const ADT_GRID_SIZE: usize = ADT_CELLS_PER_GRID * ADT_CELL_SIZE;
const WDT_MAP_SIZE: usize = 64;

const MAP_MAGIC: u32 = u32::from_le_bytes(*b"MAPS");
const MAP_VERSION_MAGIC: u32 = u32::from_le_bytes(*b"s1.4");
const MAP_AREA_MAGIC: u32 = u32::from_le_bytes(*b"AREA");
const MAP_HEIGHT_MAGIC: u32 = u32::from_le_bytes(*b"MHGT");
const MAP_LIQUID_MAGIC: u32 = u32::from_le_bytes(*b"MLIQ");

const MAP_AREA_NO_AREA: u16 = 0x0001;

const MAP_HEIGHT_NO_HEIGHT: u32 = 0x0001;
const MAP_HEIGHT_AS_INT16: u32 = 0x0002;
const MAP_HEIGHT_AS_INT8: u32 = 0x0004;

const MAP_LIQUID_NO_TYPE: u8 = 0x01;
const MAP_LIQUID_NO_HEIGHT: u8 = 0x02;

const MAP_LIQUID_TYPE_MAGMA: u8 = 0x01;
const MAP_LIQUID_TYPE_OCEAN: u8 = 0x02;
const MAP_LIQUID_TYPE_SLIME: u8 = 0x04;
const MAP_LIQUID_TYPE_WATER: u8 = 0x08;
const MAP_LIQUID_TYPE_DEEP_WATER: u8 = 0x10;

const LIQUID_TYPE_MAGMA: u16 = 0;
const LIQUID_TYPE_OCEAN: u16 = 1;
const LIQUID_TYPE_SLIME: u16 = 2;
const LIQUID_TYPE_WATER: u16 = 3;

const CONF_FLOAT_TO_INT8_LIMIT: f32 = 2.0;
const CONF_FLOAT_TO_INT16_LIMIT: f32 = 2048.0;
const CONF_FLAT_HEIGHT_DELTA_LIMIT: f32 = 0.005;
const CONF_FLAT_LIQUID_DELTA_LIMIT: f32 = 0.001;

const GRID_MAP_FILE_HEADER_SIZE: u32 = 40;
const GRID_MAP_AREA_HEADER_SIZE: u32 = 8;
const GRID_MAP_HEIGHT_HEADER_SIZE: u32 = 16;
const GRID_MAP_LIQUID_HEADER_SIZE: u32 = 16;

#[derive(Clone, Debug)]
struct MapEntry {
    id: u32,
    name: String,
}

#[derive(Clone, Copy, Debug)]
struct ExtractConfig {
    allow_height_limit: bool,
    min_height: f32,
    allow_float_to_int: bool,
}
fn ensure_dir(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

pub fn run_map_dbc(args: MapDbcArgs) -> anyhow::Result<()> {
    if args.extract_mask == 0 || args.extract_mask > (EXTRACT_MAP | EXTRACT_DBC | EXTRACT_CAMERA) {
        anyhow::bail!("Invalid extract mask: {}", args.extract_mask);
    }

    let input_path = Path::new(&args.input_path);
    if !input_path.exists() {
        anyhow::bail!("Input path does not exist: {}", args.input_path);
    }

    let output_path = Path::new(&args.output_path);
    ensure_dir(output_path)?;

    let config = ExtractConfig {
        allow_height_limit: !args.disable_min_height_limit,
        min_height: args.min_height,
        allow_float_to_int: args.float_to_int != 0,
    };

    let locales = detect_locales(input_path);
    if locales.is_empty() {
        tracing::warn!("No locales detected");
        return Ok(());
    }

    let mut first_locale: Option<usize> = None;

    if (args.extract_mask & EXTRACT_DBC) != 0 {
        for locale_idx in &locales {
            let locale = LANGS[*locale_idx];
            let mut mpq = MpqManager::new();
            load_locale_mpqs(&mut mpq, input_path, locale)?;

            let basic_locale = first_locale.is_none();
            if first_locale.is_none() {
                first_locale = Some(*locale_idx);
            }

            extract_dbc_files(&mut mpq, output_path, locale, basic_locale)?;
        }
    } else {
        first_locale = locales.first().copied();
    }

    let first_locale = match first_locale {
        Some(locale) => locale,
        None => {
            tracing::warn!("No locales detected");
            return Ok(());
        }
    };
    let locale = LANGS[first_locale];

    if (args.extract_mask & EXTRACT_CAMERA) != 0 {
        tracing::info!("Using locale: {}", locale);
        let mut mpq = MpqManager::new();
        load_locale_mpqs(&mut mpq, input_path, locale)?;
        load_common_mpqs(&mut mpq, input_path)?;
        extract_camera_files(&mut mpq, output_path, locale, true)?;
    }

    if (args.extract_mask & EXTRACT_MAP) != 0 {
        tracing::info!("Using locale: {}", locale);
        let mut mpq = MpqManager::new();
        load_locale_mpqs(&mut mpq, input_path, locale)?;
        load_common_mpqs(&mut mpq, input_path)?;
        extract_maps(&mut mpq, output_path, &config)?;
    }

    Ok(())
}

fn detect_locales(input_path: &Path) -> Vec<usize> {
    let mut locales = Vec::new();
    for (idx, locale) in LANGS.iter().enumerate() {
        let locale_mpq = format!("locale-{}.MPQ", locale);
        let path = build_path(input_path, &["Data", locale, &locale_mpq]);
        if path.exists() {
            tracing::info!("Detected locale: {}", locale);
            locales.push(idx);
        }
    }
    locales
}

fn load_locale_mpqs(mpq: &mut MpqManager, input_path: &Path, locale: &str) -> anyhow::Result<()> {
    let locale_mpq = format!("locale-{}.MPQ", locale);
    let locale_path = build_path(input_path, &["Data", locale, &locale_mpq]);
    mpq.open_archive(&locale_path)?;

    for idx in 1..=4 {
        let ext = if idx > 1 { format!("-{}", idx) } else { String::new() };
        let name = format!("patch-{}{}.MPQ", locale, ext);
        let path = build_path(input_path, &["Data", locale, &name]);
        mpq.open_archive(&path)?;
    }

    Ok(())
}

fn load_common_mpqs(mpq: &mut MpqManager, input_path: &Path) -> anyhow::Result<()> {
    for mpq_name in CONF_MPQ_LIST {
        let path = build_path(input_path, &["Data", mpq_name]);
        mpq.open_archive(&path)?;
    }
    Ok(())
}

fn mpq_to_path(base: &Path, mpq_path: &str, prefix: &str) -> PathBuf {
    let trimmed = mpq_path.strip_prefix(prefix).unwrap_or(mpq_path);
    let normalized = trimmed.replace('\\', "/");
    let mut path = base.to_path_buf();
    for part in normalized.split('/') {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}
fn extract_dbc_files(
    mpq: &mut MpqManager,
    output_path: &Path,
    locale: &str,
    basic_locale: bool,
) -> anyhow::Result<()> {
    tracing::info!("Extracting dbc files...");

    let mut dbc_files = Vec::new();
    for entry in mpq.list_files() {
        if entry.ends_with(".dbc") {
            dbc_files.push(entry);
        }
    }

    let base_path = if basic_locale {
        output_path.join("dbc")
    } else {
        output_path.join("dbc").join(locale)
    };
    ensure_dir(&base_path)?;

    let mut count = 0u32;
    for file in dbc_files {
        let Some(data) = mpq.open_file(&file) else {
            continue;
        };

        let out_path = mpq_to_path(&base_path, &file, "DBFilesClient\\");
        if let Some(parent) = out_path.parent() {
            ensure_dir(parent)?;
        }
        std::fs::write(&out_path, data)?;
        count += 1;
    }

    tracing::info!("Extracted {} DBC files", count);
    Ok(())
}

fn extract_camera_files(
    mpq: &mut MpqManager,
    output_path: &Path,
    locale: &str,
    basic_locale: bool,
) -> anyhow::Result<()> {
    tracing::info!("Extracting camera files...");

    let dbc_bytes = match mpq.open_file("DBFilesClient\\CinematicCamera.dbc") {
        Some(bytes) => bytes,
        None => {
            tracing::warn!("Unable to open CinematicCamera.dbc. Camera extract aborted.");
            return Ok(());
        }
    };

    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let base_path = if basic_locale {
        output_path.join("Cameras")
    } else {
        output_path.join("Cameras").join(locale)
    };
    ensure_dir(&base_path)?;

    let mut count = 0u32;
    for idx in 0..dbc.record_count() {
        let Some(record) = dbc.record(idx) else {
            continue;
        };
        let mut cam_file = record.get_string(1).unwrap_or_default();
        if cam_file.is_empty() {
            continue;
        }
        if let Some(pos) = cam_file.rfind(".mdx") {
            cam_file.replace_range(pos..pos + 4, ".m2");
        }

        let out_path = mpq_to_path(&base_path, &cam_file, "Cameras\\");
        if out_path.exists() {
            continue;
        }

        let Some(data) = mpq.open_file(&cam_file) else {
            continue;
        };
        if let Some(parent) = out_path.parent() {
            ensure_dir(parent)?;
        }
        std::fs::write(&out_path, data)?;
        count += 1;
    }

    tracing::info!("Extracted {} camera files", count);
    Ok(())
}
fn extract_maps(mpq: &mut MpqManager, output_path: &Path, config: &ExtractConfig) -> anyhow::Result<()> {
    tracing::info!("Extracting maps...");

    let map_ids = read_map_dbc(mpq)?;
    let (areas, max_area_id) = read_area_table_dbc(mpq)?;
    let liquid_types = read_liquid_type_dbc(mpq)?;

    let maps_path = output_path.join("maps");
    ensure_dir(&maps_path)?;

    for (index, map) in map_ids.iter().enumerate() {
        tracing::info!("Extract {} ({}/{})", map.name, index + 1, map_ids.len());

        let wdt_name = format!("World\\Maps\\{}\\{}.wdt", map.name, map.name);
        let Some(wdt_bytes) = mpq.open_file(&wdt_name) else {
            continue;
        };

        let wdt = read_wdt(&wdt_bytes)?;

        for y in 0..WDT_MAP_SIZE {
            for x in 0..WDT_MAP_SIZE {
                let Some(tile) = wdt.get_tile(x, y) else {
                    continue;
                };
                if !tile.has_adt {
                    continue;
                }

                let adt_name = format!("World\\Maps\\{}\\{}_{}_{}.adt", map.name, map.name, x, y);
                let Some(adt_bytes) = mpq.open_file(&adt_name) else {
                    continue;
                };

                let out_file = maps_path.join(format!("{:03}{:02}{:02}.map", map.id, y, x));
                convert_adt(
                    &adt_bytes,
                    &out_file,
                    &areas,
                    max_area_id,
                    &liquid_types,
                    config,
                )?;
            }
        }
    }

    Ok(())
}

fn read_wdt(data: &[u8]) -> anyhow::Result<wow_wdt::WdtFile> {
    let mut reader = WdtReader::new(Cursor::new(data), WowVersion::TBC);
    reader.read().map_err(|err| anyhow::anyhow!(err))
}

fn read_map_dbc(mpq: &mut MpqManager) -> anyhow::Result<Vec<MapEntry>> {
    tracing::info!("Read Map.dbc file...");

    let dbc_bytes = mpq
        .open_file("DBFilesClient\\Map.dbc")
        .context("Map.dbc not found")?;
    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let mut entries = Vec::with_capacity(dbc.record_count());
    for idx in 0..dbc.record_count() {
        let Some(record) = dbc.record(idx) else {
            continue;
        };
        let id = record.get_u32(0).unwrap_or(0);
        let name = record.get_string(1).unwrap_or_default();
        entries.push(MapEntry { id, name });
    }

    tracing::info!("Done! ({} maps loaded)", entries.len());
    Ok(entries)
}

fn read_area_table_dbc(mpq: &mut MpqManager) -> anyhow::Result<(Vec<u16>, u32)> {
    tracing::info!("Read AreaTable.dbc file...");

    let dbc_bytes = mpq
        .open_file("DBFilesClient\\AreaTable.dbc")
        .context("AreaTable.dbc not found")?;
    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let max_id = dbc.max_id();
    let mut areas = vec![0xffffu16; max_id as usize + 1];

    for idx in 0..dbc.record_count() {
        let Some(record) = dbc.record(idx) else {
            continue;
        };
        let id = record.get_u32(0).unwrap_or(0) as usize;
        let area_flag = record.get_u32(3).unwrap_or(0) as u16;
        if id < areas.len() {
            areas[id] = area_flag;
        }
    }

    tracing::info!("Done! ({} areas loaded)", dbc.record_count());
    Ok((areas, max_id))
}

fn read_liquid_type_dbc(mpq: &mut MpqManager) -> anyhow::Result<Vec<u16>> {
    tracing::info!("Read LiquidType.dbc file...");

    let dbc_bytes = mpq
        .open_file("DBFilesClient\\LiquidType.dbc")
        .context("LiquidType.dbc not found")?;
    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let max_id = dbc.max_id();
    let mut entries = vec![0xffffu16; max_id as usize + 1];

    for idx in 0..dbc.record_count() {
        let Some(record) = dbc.record(idx) else {
            continue;
        };
        let id = record.get_u32(0).unwrap_or(0) as usize;
        let liq_type = record.get_u32(3).unwrap_or(0) as u16;
        if id < entries.len() {
            entries[id] = liq_type;
        }
    }

    tracing::info!("Done! ({} LiqTypes loaded)", dbc.record_count());
    Ok(entries)
}
fn select_uint8_step_store(max_diff: f32) -> f32 {
    255.0 / max_diff
}

fn select_uint16_step_store(max_diff: f32) -> f32 {
    65535.0 / max_diff
}

fn idx_v8(y: usize, x: usize) -> usize {
    y * ADT_GRID_SIZE + x
}

fn idx_v9(y: usize, x: usize) -> usize {
    y * (ADT_GRID_SIZE + 1) + x
}

fn idx_cell(y: usize, x: usize) -> usize {
    y * ADT_CELLS_PER_GRID + x
}

fn convert_adt(
    adt_bytes: &[u8],
    output_path: &Path,
    areas: &[u16],
    max_area_id: u32,
    liquid_types: &[u16],
    config: &ExtractConfig,
) -> anyhow::Result<()> {
    let mut cursor = Cursor::new(adt_bytes);
    let parsed = parse_adt(&mut cursor).context("Failed to parse ADT")?;

    let root = match parsed {
        ParsedAdt::Root(root) => root,
        _ => anyhow::bail!("Unsupported ADT file type"),
    };

    let mut area_flags = vec![0xffffu16; ADT_CELLS_PER_GRID * ADT_CELLS_PER_GRID];
    let mut v8 = vec![0.0f32; ADT_GRID_SIZE * ADT_GRID_SIZE];
    let mut v9 = vec![0.0f32; (ADT_GRID_SIZE + 1) * (ADT_GRID_SIZE + 1)];

    let mut liquid_entry = vec![0u16; ADT_CELLS_PER_GRID * ADT_CELLS_PER_GRID];
    let mut liquid_flags = vec![0u8; ADT_CELLS_PER_GRID * ADT_CELLS_PER_GRID];
    let mut liquid_show = vec![0u8; ADT_GRID_SIZE * ADT_GRID_SIZE];
    let mut liquid_height = vec![0.0f32; (ADT_GRID_SIZE + 1) * (ADT_GRID_SIZE + 1)];

    let mut holes = vec![0u16; ADT_CELLS_PER_GRID * ADT_CELLS_PER_GRID];

    for i in 0..ADT_CELLS_PER_GRID {
        for j in 0..ADT_CELLS_PER_GRID {
            let idx = idx_cell(i, j);
            let Some(cell) = root.mcnk_chunks.get(idx) else {
                continue;
            };

            let area_id = cell.header.area_id;
            if area_id != 0 && area_id <= max_area_id {
                let area_flag = areas[area_id as usize];
                if area_flag != 0xffff {
                    area_flags[idx] = area_flag;
                }
            }

            holes[idx] = cell.header.holes_low_res;

            let base_height = cell.header.position[0];

            for y in 0..=ADT_CELL_SIZE {
                let cy = i * ADT_CELL_SIZE + y;
                for x in 0..=ADT_CELL_SIZE {
                    let cx = j * ADT_CELL_SIZE + x;
                    v9[idx_v9(cy, cx)] = base_height;
                }
            }

            for y in 0..ADT_CELL_SIZE {
                let cy = i * ADT_CELL_SIZE + y;
                for x in 0..ADT_CELL_SIZE {
                    let cx = j * ADT_CELL_SIZE + x;
                    v8[idx_v8(cy, cx)] = base_height;
                }
            }

            if let Some(heights) = &cell.heights {
                for y in 0..=ADT_CELL_SIZE {
                    let cy = i * ADT_CELL_SIZE + y;
                    for x in 0..=ADT_CELL_SIZE {
                        let cx = j * ADT_CELL_SIZE + x;
                        if let Some(h) = heights.get_outer_height(x, y) {
                            v9[idx_v9(cy, cx)] += h;
                        }
                    }
                }

                for y in 0..ADT_CELL_SIZE {
                    let cy = i * ADT_CELL_SIZE + y;
                    for x in 0..ADT_CELL_SIZE {
                        let cx = j * ADT_CELL_SIZE + x;
                        if let Some(h) = heights.get_inner_height(x, y) {
                            v8[idx_v8(cy, cx)] += h;
                        }
                    }
                }
            }

            if let Some(liquid) = &cell.liquid {
                let mut count = 0u32;
                for y in 0..ADT_CELL_SIZE {
                    let cy = i * ADT_CELL_SIZE + y;
                    for x in 0..ADT_CELL_SIZE {
                        let cx = j * ADT_CELL_SIZE + x;
                        let flag = liquid.get_tile_flag(x, y).unwrap_or(0x0F);
                        if flag != 0x0F {
                            liquid_show[idx_v8(cy, cx)] = 1;
                            if (flag & 0x80) != 0 {
                                liquid_flags[idx] |= MAP_LIQUID_TYPE_DEEP_WATER;
                            }
                            count += 1;
                        }
                    }
                }

                let c_flag = cell.header.flags.value;
                if (c_flag & (1 << 2)) != 0 {
                    liquid_entry[idx] = 1;
                    liquid_flags[idx] |= MAP_LIQUID_TYPE_WATER;
                }
                if (c_flag & (1 << 3)) != 0 {
                    liquid_entry[idx] = 2;
                    liquid_flags[idx] |= MAP_LIQUID_TYPE_OCEAN;
                }
                if (c_flag & (1 << 4)) != 0 {
                    liquid_entry[idx] = 3;
                    liquid_flags[idx] |= MAP_LIQUID_TYPE_MAGMA;
                }

                if count == 0 && liquid_flags[idx] != 0 {
                    tracing::warn!("Wrong liquid detect in MCLQ chunk");
                }

                for y in 0..=ADT_CELL_SIZE {
                    let cy = i * ADT_CELL_SIZE + y;
                    for x in 0..=ADT_CELL_SIZE {
                        let cx = j * ADT_CELL_SIZE + x;
                        if let Some(vertex) = liquid.get_vertex(x, y) {
                            liquid_height[idx_v9(cy, cx)] = vertex.height;
                        }
                    }
                }
            }
        }
    }

    if let Some(water) = &root.water_data {
        for i in 0..ADT_CELLS_PER_GRID {
            for j in 0..ADT_CELLS_PER_GRID {
                let idx = idx_cell(i, j);
                let Some(entry) = water.entries.get(idx) else {
                    continue;
                };
                if !entry.header.has_liquid() || entry.instances.is_empty() {
                    continue;
                }

                let instance = entry.instances[0];
                let exists_bitmap = entry.exists_bitmaps.first().copied().flatten();
                let mut show_map = exists_bitmap.unwrap_or(u64::MAX);

                let mut count = 0u32;
                for y in 0..instance.height as usize {
                    let cy = i * ADT_CELL_SIZE + y + instance.y_offset as usize;
                    for x in 0..instance.width as usize {
                        let cx = j * ADT_CELL_SIZE + x + instance.x_offset as usize;
                        let show = (show_map & 1) != 0;
                        if show {
                            liquid_show[idx_v8(cy, cx)] = 1;
                            count += 1;
                        }
                        show_map >>= 1;
                    }
                }

                liquid_entry[idx] = instance.liquid_type;
                let liq_type = liquid_types
                    .get(instance.liquid_type as usize)
                    .copied()
                    .unwrap_or(0xffff);
                match liq_type {
                    LIQUID_TYPE_WATER => liquid_flags[idx] |= MAP_LIQUID_TYPE_WATER,
                    LIQUID_TYPE_OCEAN => liquid_flags[idx] |= MAP_LIQUID_TYPE_OCEAN,
                    LIQUID_TYPE_MAGMA => liquid_flags[idx] |= MAP_LIQUID_TYPE_MAGMA,
                    LIQUID_TYPE_SLIME => liquid_flags[idx] |= MAP_LIQUID_TYPE_SLIME,
                    _ => {}
                }

                if liq_type == LIQUID_TYPE_OCEAN && let Some(attrs) = &entry.attributes {
                    let mut deep = false;
                    for y in 0..instance.height as usize {
                        for x in 0..instance.width as usize {
                            let ax = x + instance.x_offset as usize;
                            let ay = y + instance.y_offset as usize;
                            if attrs.is_deep(ax, ay) {
                                deep = true;
                                break;
                            }
                        }
                        if deep {
                            break;
                        }
                    }
                    if deep {
                        liquid_flags[idx] |= MAP_LIQUID_TYPE_DEEP_WATER;
                    }
                }

                if count == 0 && liquid_flags[idx] != 0 {
                    tracing::warn!("Wrong liquid detect in MH2O chunk");
                }

                let vdata = entry.vertex_data.first().and_then(|v| v.as_ref());
                for y in 0..=instance.height as usize {
                    let cy = i * ADT_CELL_SIZE + y + instance.y_offset as usize;
                    for x in 0..=instance.width as usize {
                        let cx = j * ADT_CELL_SIZE + x + instance.x_offset as usize;
                        let height = vertex_height(vdata, x + instance.x_offset as usize, y + instance.y_offset as usize, instance.min_height_level);
                        liquid_height[idx_v9(cy, cx)] = height;
                    }
                }
            }
        }
    }

    let (min_height, max_height) = compute_height_range(&v8, &v9);
    let (mut min_height, mut max_height) = (min_height, max_height);

    if config.allow_height_limit && min_height < config.min_height {
        clamp_heights(&mut v8, &mut v9, config.min_height);
        if min_height < config.min_height {
            min_height = config.min_height;
        }
        if max_height < config.min_height {
            max_height = config.min_height;
        }
    }

    let (height_header, height_payload) = build_height_header(&v8, &v9, min_height, max_height, config);

    let (area_header, area_payload) = build_area_header(&area_flags);

    let (liquid_header, liquid_payload) = build_liquid_header(
        &mut liquid_show,
        &mut liquid_height,
        &liquid_entry,
        &liquid_flags,
        config,
    );

    let mut map_header = GridMapFileHeader {
        map_magic: MAP_MAGIC,
        version_magic: MAP_VERSION_MAGIC,
        area_map_offset: GRID_MAP_FILE_HEADER_SIZE,
        area_map_size: GRID_MAP_AREA_HEADER_SIZE + area_payload.len() as u32,
        height_map_offset: 0,
        height_map_size: 0,
        liquid_map_offset: 0,
        liquid_map_size: 0,
        holes_offset: 0,
        holes_size: (holes.len() * 2) as u32,
    };

    map_header.height_map_offset = map_header.area_map_offset + map_header.area_map_size;
    map_header.height_map_size = GRID_MAP_HEIGHT_HEADER_SIZE + height_payload.len() as u32;

    if let Some(liquid_header) = &liquid_header {
        map_header.liquid_map_offset = map_header.height_map_offset + map_header.height_map_size;
        map_header.liquid_map_size = GRID_MAP_LIQUID_HEADER_SIZE + liquid_payload.len() as u32;
        map_header.holes_offset = map_header.liquid_map_offset + map_header.liquid_map_size;

        let sections = MapFileSections {
            area_header: &area_header,
            area_payload: &area_payload,
            height_header: &height_header,
            height_payload: &height_payload,
            liquid_header: Some(liquid_header),
            liquid_payload: &liquid_payload,
            holes: &holes,
        };
        write_map_file(output_path, &map_header, &sections)?;
    } else {
        map_header.liquid_map_offset = 0;
        map_header.liquid_map_size = 0;
        map_header.holes_offset = map_header.height_map_offset + map_header.height_map_size;

        let sections = MapFileSections {
            area_header: &area_header,
            area_payload: &area_payload,
            height_header: &height_header,
            height_payload: &height_payload,
            liquid_header: None,
            liquid_payload: &[],
            holes: &holes,
        };
        write_map_file(output_path, &map_header, &sections)?;
    }

    Ok(())
}
#[derive(Debug, Clone, Copy)]
struct GridMapFileHeader {
    map_magic: u32,
    version_magic: u32,
    area_map_offset: u32,
    area_map_size: u32,
    height_map_offset: u32,
    height_map_size: u32,
    liquid_map_offset: u32,
    liquid_map_size: u32,
    holes_offset: u32,
    holes_size: u32,
}

#[derive(Debug, Clone, Copy)]
struct GridMapAreaHeader {
    fourcc: u32,
    flags: u16,
    grid_area: u16,
}

#[derive(Debug, Clone, Copy)]
struct GridMapHeightHeader {
    fourcc: u32,
    flags: u32,
    grid_height: f32,
    grid_max_height: f32,
}

#[derive(Debug, Clone, Copy)]
struct GridMapLiquidHeader {
    fourcc: u32,
    flags: u8,
    liquid_flags: u8,
    liquid_type: u16,
    offset_x: u8,
    offset_y: u8,
    width: u8,
    height: u8,
    liquid_level: f32,
}

struct MapFileSections<'a> {
    area_header: &'a GridMapAreaHeader,
    area_payload: &'a [u8],
    height_header: &'a GridMapHeightHeader,
    height_payload: &'a [u8],
    liquid_header: Option<&'a GridMapLiquidHeader>,
    liquid_payload: &'a [u8],
    holes: &'a [u16],
}

fn compute_height_range(v8: &[f32], v9: &[f32]) -> (f32, f32) {
    let mut max_height = -20000.0f32;
    let mut min_height = 20000.0f32;

    for value in v8.iter().chain(v9.iter()) {
        if *value > max_height {
            max_height = *value;
        }
        if *value < min_height {
            min_height = *value;
        }
    }

    (min_height, max_height)
}

fn clamp_heights(v8: &mut [f32], v9: &mut [f32], min_height: f32) {
    for value in v8.iter_mut() {
        if *value < min_height {
            *value = min_height;
        }
    }
    for value in v9.iter_mut() {
        if *value < min_height {
            *value = min_height;
        }
    }
}

fn build_area_header(area_flags: &[u16]) -> (GridMapAreaHeader, Vec<u8>) {
    let first = area_flags.first().copied().unwrap_or(0xffff);
    let mut full_area = false;
    for value in area_flags {
        if *value != first {
            full_area = true;
            break;
        }
    }

    let mut header = GridMapAreaHeader {
        fourcc: MAP_AREA_MAGIC,
        flags: 0,
        grid_area: 0,
    };

    let mut payload = Vec::new();
    if full_area {
        for value in area_flags {
            payload.write_u16::<LittleEndian>(*value).unwrap();
        }
    } else {
        header.flags |= MAP_AREA_NO_AREA;
        header.grid_area = first;
    }

    (header, payload)
}

fn build_height_header(
    v8: &[f32],
    v9: &[f32],
    min_height: f32,
    max_height: f32,
    config: &ExtractConfig,
) -> (GridMapHeightHeader, Vec<u8>) {
    let mut header = GridMapHeightHeader {
        fourcc: MAP_HEIGHT_MAGIC,
        flags: 0,
        grid_height: min_height,
        grid_max_height: max_height,
    };

    if (max_height - min_height).abs() < f32::EPSILON {
        header.flags |= MAP_HEIGHT_NO_HEIGHT;
    }

    if config.allow_float_to_int && (max_height - min_height) < CONF_FLAT_HEIGHT_DELTA_LIMIT {
        header.flags |= MAP_HEIGHT_NO_HEIGHT;
    }

    let mut payload = Vec::new();
    if (header.flags & MAP_HEIGHT_NO_HEIGHT) != 0 {
        return (header, payload);
    }

    let diff = max_height - min_height;
    let mut step = 0.0f32;
    if config.allow_float_to_int {
        if diff < CONF_FLOAT_TO_INT8_LIMIT {
            header.flags |= MAP_HEIGHT_AS_INT8;
            step = select_uint8_step_store(diff);
        } else if diff < CONF_FLOAT_TO_INT16_LIMIT {
            header.flags |= MAP_HEIGHT_AS_INT16;
            step = select_uint16_step_store(diff);
        }
    }

    if (header.flags & MAP_HEIGHT_AS_INT8) != 0 {
        for value in v9 {
            let packed = ((value - min_height) * step + 0.5).clamp(0.0, 255.0) as u8;
            payload.write_u8(packed).unwrap();
        }
        for value in v8 {
            let packed = ((value - min_height) * step + 0.5).clamp(0.0, 255.0) as u8;
            payload.write_u8(packed).unwrap();
        }
    } else if (header.flags & MAP_HEIGHT_AS_INT16) != 0 {
        for value in v9 {
            let packed = ((value - min_height) * step + 0.5).clamp(0.0, 65535.0) as u16;
            payload.write_u16::<LittleEndian>(packed).unwrap();
        }
        for value in v8 {
            let packed = ((value - min_height) * step + 0.5).clamp(0.0, 65535.0) as u16;
            payload.write_u16::<LittleEndian>(packed).unwrap();
        }
    } else {
        for value in v9 {
            payload.write_f32::<LittleEndian>(*value).unwrap();
        }
        for value in v8 {
            payload.write_f32::<LittleEndian>(*value).unwrap();
        }
    }

    (header, payload)
}

fn build_liquid_header(
    liquid_show: &mut [u8],
    liquid_height: &mut [f32],
    liquid_entry: &[u16],
    liquid_flags: &[u8],
    config: &ExtractConfig,
) -> (Option<GridMapLiquidHeader>, Vec<u8>) {
    let first_entry = liquid_entry.first().copied().unwrap_or(0);
    let first_flag = liquid_flags.first().copied().unwrap_or(0);

    let mut full_type = false;
    for (entry, flag) in liquid_entry.iter().zip(liquid_flags.iter()) {
        if *entry != first_entry || *flag != first_flag {
            full_type = true;
            break;
        }
    }

    if first_flag == 0 && !full_type {
        return (None, Vec::new());
    }

    let mut min_x = 255usize;
    let mut min_y = 255usize;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    let mut max_height = -20000.0f32;
    let mut min_height = 20000.0f32;

    for y in 0..ADT_GRID_SIZE {
        for x in 0..ADT_GRID_SIZE {
            let idx = idx_v8(y, x);
            if liquid_show[idx] != 0 {
                if min_x > x {
                    min_x = x;
                }
                if max_x < x {
                    max_x = x;
                }
                if min_y > y {
                    min_y = y;
                }
                if max_y < y {
                    max_y = y;
                }
                let h = liquid_height[idx_v9(y, x)];
                if max_height < h {
                    max_height = h;
                }
                if min_height > h {
                    min_height = h;
                }
            } else {
                liquid_height[idx_v9(y, x)] = config.min_height;
                if min_height > config.min_height {
                    min_height = config.min_height;
                }
            }
        }
    }

    if min_x == 255 || min_y == 255 {
        min_x = 0;
        min_y = 0;
        max_x = 0;
        max_y = 0;
    }

    let mut header = GridMapLiquidHeader {
        fourcc: MAP_LIQUID_MAGIC,
        flags: 0,
        liquid_flags: 0,
        liquid_type: 0,
        offset_x: min_x as u8,
        offset_y: min_y as u8,
        width: (max_x - min_x + 2) as u8,
        height: (max_y - min_y + 2) as u8,
        liquid_level: min_height,
    };

    if (max_height - min_height).abs() < f32::EPSILON {
        header.flags |= MAP_LIQUID_NO_HEIGHT;
    }

    if config.allow_float_to_int && (max_height - min_height) < CONF_FLAT_LIQUID_DELTA_LIMIT {
        header.flags |= MAP_LIQUID_NO_HEIGHT;
    }

    if !full_type {
        header.flags |= MAP_LIQUID_NO_TYPE;
    }

    let mut payload = Vec::new();

    if (header.flags & MAP_LIQUID_NO_TYPE) != 0 {
        header.liquid_flags = first_flag;
        header.liquid_type = first_entry;
    } else {
        for entry in liquid_entry {
            payload.write_u16::<LittleEndian>(*entry).unwrap();
        }
        for flag in liquid_flags {
            payload.write_u8(*flag).unwrap();
        }
    }

    if (header.flags & MAP_LIQUID_NO_HEIGHT) == 0 {
        for y in 0..header.height as usize {
            let row = y + header.offset_y as usize;
            let start = idx_v9(row, header.offset_x as usize);
            let end = start + header.width as usize;
            for value in &liquid_height[start..end] {
                payload.write_f32::<LittleEndian>(*value).unwrap();
            }
        }
    }

    (Some(header), payload)
}

fn vertex_height(vdata: Option<&VertexDataArray>, x: usize, y: usize, min_height: f32) -> f32 {
    if x > 8 || y > 8 {
        return min_height;
    }
    let idx = y * 9 + x;
    let Some(vdata) = vdata else {
        return min_height;
    };

    match vdata {
        VertexDataArray::HeightDepth(grid) => grid[idx]
            .map(|v| v.absolute_height(min_height))
            .unwrap_or(min_height),
        VertexDataArray::HeightUv(grid) => grid[idx]
            .map(|v| v.absolute_height(min_height))
            .unwrap_or(min_height),
        VertexDataArray::DepthOnly(_) => min_height,
        VertexDataArray::HeightUvDepth(grid) => grid[idx]
            .map(|v| v.absolute_height(min_height))
            .unwrap_or(min_height),
    }
}

fn write_map_file(
    output_path: &Path,
    map_header: &GridMapFileHeader,
    sections: &MapFileSections<'_>,
) -> anyhow::Result<()> {
    if let Some(parent) = output_path.parent() {
        ensure_dir(parent)?;
    }

    let mut file = std::fs::File::create(output_path)?;

    file.write_u32::<LittleEndian>(map_header.map_magic)?;
    file.write_u32::<LittleEndian>(map_header.version_magic)?;
    file.write_u32::<LittleEndian>(map_header.area_map_offset)?;
    file.write_u32::<LittleEndian>(map_header.area_map_size)?;
    file.write_u32::<LittleEndian>(map_header.height_map_offset)?;
    file.write_u32::<LittleEndian>(map_header.height_map_size)?;
    file.write_u32::<LittleEndian>(map_header.liquid_map_offset)?;
    file.write_u32::<LittleEndian>(map_header.liquid_map_size)?;
    file.write_u32::<LittleEndian>(map_header.holes_offset)?;
    file.write_u32::<LittleEndian>(map_header.holes_size)?;

    file.write_u32::<LittleEndian>(sections.area_header.fourcc)?;
    file.write_u16::<LittleEndian>(sections.area_header.flags)?;
    file.write_u16::<LittleEndian>(sections.area_header.grid_area)?;
    file.write_all(sections.area_payload)?;

    file.write_u32::<LittleEndian>(sections.height_header.fourcc)?;
    file.write_u32::<LittleEndian>(sections.height_header.flags)?;
    file.write_f32::<LittleEndian>(sections.height_header.grid_height)?;
    file.write_f32::<LittleEndian>(sections.height_header.grid_max_height)?;
    file.write_all(sections.height_payload)?;

    if let Some(liquid_header) = sections.liquid_header {
        file.write_u32::<LittleEndian>(liquid_header.fourcc)?;
        file.write_u8(liquid_header.flags)?;
        file.write_u8(liquid_header.liquid_flags)?;
        file.write_u16::<LittleEndian>(liquid_header.liquid_type)?;
        file.write_u8(liquid_header.offset_x)?;
        file.write_u8(liquid_header.offset_y)?;
        file.write_u8(liquid_header.width)?;
        file.write_u8(liquid_header.height)?;
        file.write_f32::<LittleEndian>(liquid_header.liquid_level)?;
        file.write_all(sections.liquid_payload)?;
    }

    for hole in sections.holes {
        file.write_u16::<LittleEndian>(*hole)?;
    }

    file.flush()?;
    Ok(())
}
