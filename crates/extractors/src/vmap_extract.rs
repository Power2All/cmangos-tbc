use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use wow_adt::{parse_adt, ParsedAdt};
use wow_wdt::{version::WowVersion, WdtReader};

use crate::dbc::DbcFile;
use crate::mpq::{build_path, MpqManager};
use crate::VmapExtractArgs;

const VMAP_MAGIC: &[u8; 8] = b"VMAPs05\0";
const BUILDINGS_DIR: &str = "Buildings";
const DIR_BIN: &str = "dir_bin";
const TEMP_GAMEOBJECT_LIST: &str = "temp_gameobject_models";

const LANGS: [&str; 12] = [
    "enGB", "enUS", "deDE", "esES", "frFR", "koKR", "zhCN", "zhTW", "enCN", "enTW", "esMX", "ruRU",
];

const WMO_MATERIAL_COLLISION: u8 = 0x08;
const WMO_MATERIAL_DETAIL: u8 = 0x04;
const WMO_MATERIAL_RENDER: u8 = 0x20;

const MOD_M2: u32 = 1;
const MOD_WORLDSPAWN: u32 = 1 << 1;
const MOD_HAS_BOUND: u32 = 1 << 2;

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
}

#[derive(Clone, Copy, Debug, Default)]
struct Quaternion {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct AaBox {
    min: Vec3,
    max: Vec3,
}

#[derive(Clone, Debug)]
struct WmoDoodadSet {
    name: [u8; 20],
    start_index: u32,
    count: u32,
    _pad: [u8; 4],
}

#[derive(Clone, Debug)]
struct WmoDoodadSpawn {
    name_index: u32,
    position: Vec3,
    rotation: Quaternion,
    scale: f32,
    color: u32,
}

#[derive(Clone, Debug, Default)]
struct WmoDoodadData {
    sets: Vec<WmoDoodadSet>,
    paths_blob: Vec<u8>,
    spawns: Vec<WmoDoodadSpawn>,
    references: HashSet<u16>,
}

#[derive(Debug)]
struct WmoRoot {
    n_groups: u32,
    root_wmo_id: u32,
    flags: u32,
    bbcorn1: [f32; 3],
    bbcorn2: [f32; 3],
    doodad_data: WmoDoodadData,
    valid_doodad_names: HashSet<u32>,
    group_names: Vec<u8>,
}

#[derive(Debug, Default)]
struct WmoLiquidHeader {
    xverts: i32,
    yverts: i32,
    xtiles: i32,
    ytiles: i32,
    pos_x: f32,
    pos_y: f32,
    pos_z: f32,
    liquid_type: i16,
}

#[derive(Debug, Default)]
struct WmoLiquidVert {
    _unk1: u16,
    _unk2: u16,
    height: f32,
}

#[derive(Debug)]
struct WmoGroup {
    group_name: i32,
    desc_group_name: i32,
    mogp_flags: i32,
    bbcorn1: [f32; 3],
    bbcorn2: [f32; 3],
    mopr_idx: u16,
    mopr_n_items: u16,
    n_batch_a: u16,
    n_batch_b: u16,
    n_batch_c: u32,
    fog_idx: u32,
    liquid_type: u32,
    group_wmo_id: u32,
    mopy: Vec<u8>,
    movi: Vec<u16>,
    movt: Vec<f32>,
    moba: Vec<u16>,
    doodad_refs: Vec<u16>,
    liquid_header: Option<WmoLiquidHeader>,
    liquid_verts: Vec<WmoLiquidVert>,
    liquid_bytes: Vec<u8>,
    liquflags: u32,
}

#[derive(Default)]
struct UniqueIds {
    map: HashMap<(u32, u16), u32>,
}

impl UniqueIds {
    fn generate(&mut self, client_id: u32, doodad_id: u16) -> u32 {
        let key = (client_id, doodad_id);
        if let Some(value) = self.map.get(&key) {
            return *value;
        }
        let next = (self.map.len() + 1) as u32;
        self.map.insert(key, next);
        next
    }
}

struct VmapContext {
    mpq: MpqManager,
    output_root: PathBuf,
    buildings_dir: PathBuf,
    precise: bool,
    unique_ids: UniqueIds,
    wmo_doodads: HashMap<String, WmoDoodadData>,
    failed_paths: HashSet<String>,
}

pub fn run_vmap_extract(args: VmapExtractArgs) -> anyhow::Result<()> {
    let data_path = Path::new(&args.data_path);
    if !data_path.exists() {
        anyhow::bail!("Data path does not exist: {}", args.data_path);
    }

    let output_root = PathBuf::from(&args.output_path);
    if !output_root.exists() {
        std::fs::create_dir_all(&output_root)?;
    }

    let buildings_dir = output_root.join(BUILDINGS_DIR);
    let dirty_dir = buildings_dir.join("dir");
    let dirty_dir_bin = buildings_dir.join(DIR_BIN);
    if dirty_dir.exists() || dirty_dir_bin.exists() {
        anyhow::bail!("Your output directory seems to be polluted, please use an empty directory!");
    }

    if !buildings_dir.exists() {
        std::fs::create_dir_all(&buildings_dir)?;
    }

    let precise = args.large && !args.small;

    let mut mpq = MpqManager::new();
    let archives = build_archive_list(data_path)?;
    for archive in archives {
        let path = build_path(data_path, &[&archive]);
        mpq.open_archive(&path)?;
    }

    if mpq.list_files().is_empty() {
        anyhow::bail!(
            "FATAL ERROR: None MPQ archive found by path '{}'. Use -d option with proper path.",
            args.data_path
        );
    }

    let mut context = VmapContext {
        mpq,
        output_root,
        buildings_dir,
        precise,
        unique_ids: UniqueIds::default(),
        wmo_doodads: HashMap::new(),
        failed_paths: HashSet::new(),
    };

    tracing::info!("Extract for VMAPs05. Beginning work ....");

    extract_wmos(&mut context)?;

    let maps = read_map_dbc(&mut context)?;
    parse_maps(&mut context, &maps)?;

    extract_gameobject_models(&mut context)?;

    if !context.failed_paths.is_empty() {
        tracing::warn!("Some models could not be extracted:");
        for path in &context.failed_paths {
            tracing::warn!("Could not find file of model {}", path);
        }
    }

    tracing::info!("Extract for VMAPs05. Work complete. No errors.");
    Ok(())
}
fn build_archive_list(data_path: &Path) -> anyhow::Result<Vec<String>> {
    let mut archives = Vec::new();
    let mut locales = Vec::new();

    for locale in LANGS {
        let locale_path = data_path.join(locale);
        if locale_path.is_dir() {
            tracing::info!("Found locale '{}'", locale);
            locales.push(locale.to_string());
        }
    }

    if locales.is_empty() {
        return Ok(archives);
    }

    tracing::info!("Adding data files from locale directories.");
    for locale in &locales {
        archives.push(format!("{}/locale-{}.MPQ", locale, locale));
        archives.push(format!("{}/expansion-locale-{}.MPQ", locale, locale));
    }

    archives.push("common.MPQ".to_string());
    archives.push("expansion.MPQ".to_string());

    tracing::info!("Scanning patch levels from data directory.");
    scan_patches(data_path, "patch", &mut archives);

    tracing::info!("Scanning patch levels from locale directories.");
    for locale in &locales {
        scan_patches(data_path, &format!("{}/patch-{}", locale, locale), &mut archives);
    }

    Ok(archives)
}

fn scan_patches(base: &Path, stem: &str, archives: &mut Vec<String>) {
    for idx in 1..=99 {
        let name = if idx == 1 {
            format!("{}.MPQ", stem)
        } else {
            format!("{}-{}.MPQ", stem, idx)
        };
        if base.join(&name).exists() {
            archives.push(name);
        }
    }
}

fn get_plain_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn get_extension(path: &str) -> Option<&str> {
    Path::new(path).extension().and_then(OsStr::to_str)
}

fn fixnamen(name: &mut [u8]) {
    if name.len() < 3 {
        return;
    }

    let len = name.len();
    for i in 0..len - 3 {
        let c = name[i];
        let prev = if i > 0 { name[i - 1] } else { 0 };
        if i > 0 && c.is_ascii_uppercase() && prev.is_ascii_alphabetic() {
            name[i] = c.to_ascii_lowercase();
        } else if (i == 0 || !prev.is_ascii_alphabetic()) && c.is_ascii_lowercase() {
            name[i] = c.to_ascii_uppercase();
        }
    }

    for i in len - 3..len {
        name[i] = name[i].to_ascii_lowercase();
    }
}

fn fixname2(name: &mut [u8]) {
    for c in name.iter_mut() {
        if *c == b' ' {
            *c = b'_';
        }
    }
}

fn normalize_filename(name: &str) -> String {
    let mut bytes = name.as_bytes().to_vec();
    fixnamen(&mut bytes);
    let fixed = String::from_utf8_lossy(&bytes);
    let plain = get_plain_name(&fixed);
    let mut plain_bytes = plain.as_bytes().to_vec();
    fixname2(&mut plain_bytes);
    String::from_utf8_lossy(&plain_bytes).to_string()
}

fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn fix_coords(v: Vec3) -> Vec3 {
    Vec3::new(v.z, v.x, v.y)
}

fn fix_coord_system(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

fn deg_to_rad(value: f32) -> f32 {
    value * std::f32::consts::PI / 180.0
}

fn rad_to_deg(value: f32) -> f32 {
    value * 180.0 / std::f32::consts::PI
}

fn quat_to_matrix(q: Quaternion) -> [[f32; 3]; 3] {
    let x2 = q.x + q.x;
    let y2 = q.y + q.y;
    let z2 = q.z + q.z;
    let xx = q.x * x2;
    let yy = q.y * y2;
    let zz = q.z * z2;
    let xy = q.x * y2;
    let xz = q.x * z2;
    let yz = q.y * z2;
    let wx = q.w * x2;
    let wy = q.w * y2;
    let wz = q.w * z2;

    [
        [1.0 - (yy + zz), xy - wz, xz + wy],
        [xy + wz, 1.0 - (xx + zz), yz - wx],
        [xz - wy, yz + wx, 1.0 - (xx + yy)],
    ]
}

fn matrix_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
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

/// Extract Euler angles from a rotation matrix using the XYZ convention.
/// Matches G3D::Matrix3::toEulerAnglesXYZ exactly:
///   X-angle = atan2(-m[1][2], m[2][2])
///   Y-angle = asin(m[0][2])
///   Z-angle = atan2(-m[0][1], m[0][0])
fn matrix_to_euler_xyz(m: [[f32; 3]; 3]) -> (f32, f32, f32) {
    let sy = m[0][2].clamp(-1.0, 1.0);
    let y = sy.asin();
    let cy = y.cos();

    let (x, z) = if cy.abs() > 1e-6 {
        let x = (-m[1][2]).atan2(m[2][2]);
        let z = (-m[0][1]).atan2(m[0][0]);
        (x, z)
    } else {
        // Gimbal lock
        if m[0][2] < 0.0 {
            // Y near -pi/2
            let x = (-m[1][0]).atan2(m[1][1]);
            (x, 0.0)
        } else {
            // Y near +pi/2
            let x = m[1][0].atan2(m[1][1]);
            (x, 0.0)
        }
    };

    (x, y, z)
}
struct MpqFile {
    data: Vec<u8>,
    pos: usize,
}

impl MpqFile {
    fn open(mpq: &mut MpqManager, filename: &str) -> Option<Self> {
        let data = mpq.open_file(filename)?;
        if data.len() <= 1 {
            return None;
        }
        Some(Self { data, pos: 0 })
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> anyhow::Result<()> {
        let end = self.pos + buf.len();
        if end > self.data.len() {
            anyhow::bail!("Unexpected EOF");
        }
        buf.copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        Ok(())
    }

    fn read_u32(&mut self) -> anyhow::Result<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn read_i32(&mut self) -> anyhow::Result<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    fn read_u16(&mut self) -> anyhow::Result<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    fn read_i16(&mut self) -> anyhow::Result<i16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(i16::from_le_bytes(buf))
    }

    fn read_f32(&mut self) -> anyhow::Result<f32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(f32::from_le_bytes(buf))
    }

    fn read_vec(&mut self, size: usize) -> anyhow::Result<Vec<u8>> {
        let mut buf = vec![0u8; size];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    fn position(&self) -> usize {
        self.pos
    }

    fn get_slice(&self, offset: usize, size: usize) -> Option<&[u8]> {
        let end = offset + size;
        if end > self.data.len() {
            return None;
        }
        Some(&self.data[offset..end])
    }
}

fn flip_fourcc(mut fcc: [u8; 4]) -> [u8; 4] {
    fcc.swap(0, 3);
    fcc.swap(1, 2);
    fcc
}

fn read_chunk_header(file: &mut MpqFile) -> anyhow::Result<(String, u32, usize)> {
    let mut fcc = [0u8; 4];
    if file.is_eof() {
        anyhow::bail!("EOF");
    }
    file.read_exact(&mut fcc)?;
    let size = file.read_u32()?;
    let fcc = flip_fourcc(fcc);
    let name = String::from_utf8_lossy(&fcc).to_string();
    let next_pos = file.position() + size as usize;
    Ok((name, size, next_pos))
}
#[derive(Debug, Default)]
struct ModelHeaderInfo {
    n_bounding_triangles: u32,
    ofs_bounding_triangles: u32,
    n_bounding_vertices: u32,
    ofs_bounding_vertices: u32,
}

fn parse_model_header(data: &[u8]) -> anyhow::Result<ModelHeaderInfo> {
    let mut cursor = Cursor::new(data);
    let mut skip_buf = [0u8; 4];

    cursor.read_exact(&mut skip_buf)?; // id
    cursor.read_exact(&mut skip_buf)?; // version

    let read_u32 = |c: &mut Cursor<&[u8]>| -> anyhow::Result<u32> {
        let mut buf = [0u8; 4];
        c.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    };

    for _ in 0..43 {
        read_u32(&mut cursor)?; // nameLength..ofsTexAnimLookup
    }

    for _ in 0..14 {
        let mut buf = [0u8; 4];
        cursor.read_exact(&mut buf)?; // floats
    }

    let n_bounding_triangles = read_u32(&mut cursor)?;
    let ofs_bounding_triangles = read_u32(&mut cursor)?;
    let n_bounding_vertices = read_u32(&mut cursor)?;
    let ofs_bounding_vertices = read_u32(&mut cursor)?;

    Ok(ModelHeaderInfo {
        n_bounding_triangles,
        ofs_bounding_triangles,
        n_bounding_vertices,
        ofs_bounding_vertices,
    })
}

fn extract_single_model(
    context: &mut VmapContext,
    orig_path: &str,
) -> anyhow::Result<Option<String>> {
    if orig_path.len() < 4 {
        return Ok(None);
    }

    let mut path = orig_path.to_string();
    if let Some(ext) = get_extension(get_plain_name(&path)) {
        if ext.eq_ignore_ascii_case("mdx") || ext.eq_ignore_ascii_case("mdl") {
            path.truncate(path.len().saturating_sub(2));
            path.push('2');
        }
    }

    let fixed_name = get_plain_name(&path).to_string();
    let output = context.buildings_dir.join(&fixed_name);
    if output.exists() {
        return Ok(Some(fixed_name));
    }

    let Some(file) = MpqFile::open(&mut context.mpq, &path) else {
        context.failed_paths.insert(path);
        return Ok(None);
    };

    let header = parse_model_header(&file.data)?;
    if header.n_bounding_triangles == 0 {
        return Ok(None);
    }

    let vertices_offset = header.ofs_bounding_vertices as usize;
    let vertices_size = header.n_bounding_vertices as usize * 12;
    let Some(vertices_slice) = file.get_slice(vertices_offset, vertices_size) else {
        return Ok(None);
    };

    let indices_offset = header.ofs_bounding_triangles as usize;
    let indices_size = header.n_bounding_triangles as usize * 2;
    let Some(indices_slice) = file.get_slice(indices_offset, indices_size) else {
        return Ok(None);
    };

    let mut vertices = Vec::with_capacity(header.n_bounding_vertices as usize);
    let mut cursor = Cursor::new(vertices_slice);
    for _ in 0..header.n_bounding_vertices {
        let x = cursor.read_f32::<LittleEndian>()?;
        let y = cursor.read_f32::<LittleEndian>()?;
        let z = cursor.read_f32::<LittleEndian>()?;
        vertices.push(fix_coord_system(Vec3::new(x, y, z)));
    }

    let mut indices = Vec::with_capacity(header.n_bounding_triangles as usize);
    let mut cursor = Cursor::new(indices_slice);
    for _ in 0..header.n_bounding_triangles {
        indices.push(cursor.read_u16::<LittleEndian>()?);
    }

    let mut output_file = std::fs::File::create(&output)?;
    output_file.write_all(VMAP_MAGIC)?;
    let n_vertices = header.n_bounding_vertices as u32;
    output_file.write_u32::<LittleEndian>(n_vertices)?;
    output_file.write_u32::<LittleEndian>(1)?;

    let zeros = [0u8; 12];
    output_file.write_all(&zeros)?;
    let zeros = [0u8; 24];
    output_file.write_all(&zeros)?;
    let zeros = [0u8; 4];
    output_file.write_all(&zeros)?;

    output_file.write_all(b"GRP ")?;
    let branches = 1u32;
    let wsize = std::mem::size_of::<u32>() as u32 + std::mem::size_of::<u32>() as u32 * branches;
    output_file.write_u32::<LittleEndian>(wsize)?;
    output_file.write_u32::<LittleEndian>(branches)?;

    let n_indexes = header.n_bounding_triangles;
    output_file.write_u32::<LittleEndian>(n_indexes)?;

    output_file.write_all(b"INDX")?;
    let wsize = std::mem::size_of::<u32>() as u32 + std::mem::size_of::<u16>() as u32 * n_indexes;
    output_file.write_u32::<LittleEndian>(wsize)?;
    output_file.write_u32::<LittleEndian>(n_indexes)?;

    if n_indexes > 0 {
        for i in 0..indices.len() {
            if i % 3 == 1 && i + 1 < indices.len() {
                indices.swap(i, i + 1);
            }
        }
        for idx in indices {
            output_file.write_u16::<LittleEndian>(idx)?;
        }
    }

    output_file.write_all(b"VERT")?;
    let wsize = std::mem::size_of::<u32>() as u32 + 12 * n_vertices;
    output_file.write_u32::<LittleEndian>(wsize)?;
    output_file.write_u32::<LittleEndian>(n_vertices)?;

    for mut v in vertices {
        let tmp = v.y;
        v.y = -v.z;
        v.z = tmp;
        output_file.write_f32::<LittleEndian>(v.x)?;
        output_file.write_f32::<LittleEndian>(v.y)?;
        output_file.write_f32::<LittleEndian>(v.z)?;
    }

    output_file.flush()?;

    Ok(Some(fixed_name))
}
fn extract_wmos(context: &mut VmapContext) -> anyhow::Result<()> {
    let mut wmo_files: Vec<String> = context
        .mpq
        .list_files()
        .into_iter()
        .filter(|name| name.to_ascii_lowercase().ends_with(".wmo"))
        .collect();
    wmo_files.sort();

    for name in wmo_files {
        let mut fname = name;
        extract_single_wmo(context, &mut fname)?;
    }

    Ok(())
}

fn extract_single_wmo(context: &mut VmapContext, fname: &mut String) -> anyhow::Result<bool> {
    let plain_name = get_plain_name(fname);
    let fixed = normalize_filename(plain_name);

    let local_path = context.buildings_dir.join(&fixed);
    if local_path.exists() {
        return Ok(true);
    }

    if let Some(pos) = fixed.rfind('_') {
        let suffix = &fixed[pos..];
        let chars: Vec<char> = suffix.chars().collect();
        if chars.len() >= 4 {
            let digit_count = chars[1..4].iter().filter(|c| c.is_ascii_digit()).count();
            if digit_count == 3 {
                return Ok(true);
            }
        }
    }

    let Some(root) = WmoRoot::open(context, fname)? else {
        return Ok(true);
    };

    let mut output = std::fs::File::create(&local_path)?;
    root.write_header(&mut output)?;

    let mut root = root;
    let mut doodads = std::mem::take(&mut root.doodad_data);

    let mut total_triangles = 0u32;
    let mut real_groups = root.n_groups;

    for idx in 0..root.n_groups {
        let mut group_name = fname.clone();
        if group_name.len() < 4 {
            continue;
        }
        group_name.truncate(group_name.len() - 4);
        group_name.push_str(&format!("_{:03}.wmo", idx));

        let Some(mut group) = WmoGroup::open(context, &group_name)? else {
            tracing::warn!("Could not open all Group file for: {}", fixed);
            real_groups = real_groups.saturating_sub(1);
            continue;
        };

        if group.should_skip(&root) {
            real_groups = real_groups.saturating_sub(1);
            continue;
        }

        let group_triangles = group.write_group(&mut output, &root, context.precise, &group_name)?;
        total_triangles = total_triangles.saturating_add(group_triangles);

        for reference in &group.doodad_refs {
            if (*reference as usize) >= doodads.spawns.len() {
                continue;
            }
            let doodad_name_index = doodads.spawns[*reference as usize].name_index;
            if root.valid_doodad_names.contains(&doodad_name_index) {
                doodads.references.insert(*reference);
            }
        }
    }

    output.seek(SeekFrom::Start(8))?;
    output.write_u32::<LittleEndian>(total_triangles)?;
    output.seek(SeekFrom::Start(12))?;
    output.write_u32::<LittleEndian>(real_groups)?;
    output.flush()?;

    context.wmo_doodads.insert(fixed, doodads);

    Ok(true)
}

impl WmoRoot {
    fn open(context: &mut VmapContext, filename: &str) -> anyhow::Result<Option<Self>> {
        let Some(mut file) = MpqFile::open(&mut context.mpq, filename) else {
            return Ok(None);
        };

        let mut root = WmoRoot {
            n_groups: 0,
            root_wmo_id: 0,
            flags: 0,
            bbcorn1: [0.0; 3],
            bbcorn2: [0.0; 3],
            doodad_data: WmoDoodadData::default(),
            valid_doodad_names: HashSet::new(),
            group_names: Vec::new(),
        };

        while !file.is_eof() {
            let (fourcc, size, nextpos) = match read_chunk_header(&mut file) {
                Ok(header) => header,
                Err(_) => break,
            };

            match fourcc.as_str() {
                "MOHD" => {
                    let _n_textures = file.read_u32()?;
                    root.n_groups = file.read_u32()?;
                    let _n_portals = file.read_u32()?;
                    let _n_lights = file.read_u32()?;
                    let _n_doodad_names = file.read_u32()?;
                    let _n_doodad_defs = file.read_u32()?;
                    let _n_doodad_sets = file.read_u32()?;
                    let _color = file.read_u32()?;
                    root.root_wmo_id = file.read_u32()?;
                    root.bbcorn1 = [file.read_f32()?, file.read_f32()?, file.read_f32()?];
                    root.bbcorn2 = [file.read_f32()?, file.read_f32()?, file.read_f32()?];
                    root.flags = file.read_u32()?;
                }
                "MODS" => {
                    let count = size as usize / 32;
                    let mut sets = Vec::with_capacity(count);
                    for _ in 0..count {
                        let mut name = [0u8; 20];
                        file.read_exact(&mut name)?;
                        let start_index = file.read_u32()?;
                        let count = file.read_u32()?;
                        let mut pad = [0u8; 4];
                        file.read_exact(&mut pad)?;
                        sets.push(WmoDoodadSet {
                            name,
                            start_index,
                            count,
                            _pad: pad,
                        });
                    }
                    root.doodad_data.sets = sets;
                }
                "MODN" => {
                    let mut data = file.read_vec(size as usize)?;
                    let mut offset = 0usize;
                    while offset < data.len() {
                        let Some(end) = data[offset..].iter().position(|b| *b == 0) else {
                            break;
                        };
                        let end_pos = offset + end;
                        let path = String::from_utf8_lossy(&data[offset..end_pos]).to_string();

                        if let Some(name_start) = data[offset..end_pos]
                            .iter()
                            .rposition(|b| *b == b'\\' || *b == b'/')
                        {
                            let name_offset = offset + name_start + 1;
                            let name_len = end_pos.saturating_sub(name_offset);
                            if name_len > 0 {
                                fixnamen(&mut data[name_offset..end_pos]);
                                fixname2(&mut data[name_offset..end_pos]);
                            }
                        }

                        let doodad_name_index = offset as u32;
                        if let Some(fixed_name) = extract_single_model(context, &path)? {
                            if !fixed_name.is_empty() {
                                root.valid_doodad_names.insert(doodad_name_index);
                            }
                        }

                        offset = end_pos + 1;
                    }

                    root.doodad_data.paths_blob = data;
                }
                "MODD" => {
                    let count = size as usize / 40;
                    let mut spawns = Vec::with_capacity(count);
                    for _ in 0..count {
                        let raw_name = file.read_u32()?;
                        let name_index = raw_name & 0x00FF_FFFF;
                        let position = Vec3::new(file.read_f32()?, file.read_f32()?, file.read_f32()?);
                        let rotation = Quaternion {
                            x: file.read_f32()?,
                            y: file.read_f32()?,
                            z: file.read_f32()?,
                            w: file.read_f32()?,
                        };
                        let scale = file.read_f32()?;
                        let color = file.read_u32()?;
                        spawns.push(WmoDoodadSpawn {
                            name_index,
                            position,
                            rotation,
                            scale,
                            color,
                        });
                    }
                    root.doodad_data.spawns = spawns;
                }
                "MOGN" => {
                    root.group_names = file.read_vec(size as usize)?;
                }
                _ => {}
            }

            file.seek(nextpos);
        }

        Ok(Some(root))
    }

    fn write_header(&self, out: &mut std::fs::File) -> anyhow::Result<()> {
        out.write_all(VMAP_MAGIC)?;
        out.write_u32::<LittleEndian>(0)?;
        out.write_u32::<LittleEndian>(self.n_groups)?;
        out.write_u32::<LittleEndian>(self.root_wmo_id)?;
        Ok(())
    }
}

impl WmoGroup {
    fn open(context: &mut VmapContext, filename: &str) -> anyhow::Result<Option<Self>> {
        let Some(mut file) = MpqFile::open(&mut context.mpq, filename) else {
            return Ok(None);
        };

        let mut group = WmoGroup {
            group_name: 0,
            desc_group_name: 0,
            mogp_flags: 0,
            bbcorn1: [0.0; 3],
            bbcorn2: [0.0; 3],
            mopr_idx: 0,
            mopr_n_items: 0,
            n_batch_a: 0,
            n_batch_b: 0,
            n_batch_c: 0,
            fog_idx: 0,
            liquid_type: 0,
            group_wmo_id: 0,
            mopy: Vec::new(),
            movi: Vec::new(),
            movt: Vec::new(),
            moba: Vec::new(),
            doodad_refs: Vec::new(),
            liquid_header: None,
            liquid_verts: Vec::new(),
            liquid_bytes: Vec::new(),
            liquflags: 0,
        };

        while !file.is_eof() {
            let (fourcc, mut size, nextpos) = match read_chunk_header(&mut file) {
                Ok(header) => header,
                Err(_) => break,
            };

            if fourcc == "MOGP" {
                size = 68;
            }

            match fourcc.as_str() {
                "MOGP" => {
                    group.group_name = file.read_i32()?;
                    group.desc_group_name = file.read_i32()?;
                    group.mogp_flags = file.read_i32()?;
                    group.bbcorn1 = [file.read_f32()?, file.read_f32()?, file.read_f32()?];
                    group.bbcorn2 = [file.read_f32()?, file.read_f32()?, file.read_f32()?];
                    group.mopr_idx = file.read_u16()?;
                    group.mopr_n_items = file.read_u16()?;
                    group.n_batch_a = file.read_u16()?;
                    group.n_batch_b = file.read_u16()?;
                    group.n_batch_c = file.read_u32()?;
                    group.fog_idx = file.read_u32()?;
                    group.liquid_type = file.read_u32()?;
                    group.group_wmo_id = file.read_u32()?;
                }
                "MOPY" => {
                    group.mopy = file.read_vec(size as usize)?;
                }
                "MOVI" => {
                    let mut data = Vec::with_capacity(size as usize / 2);
                    for _ in 0..(size / 2) {
                        data.push(file.read_u16()?);
                    }
                    group.movi = data;
                }
                "MOVT" => {
                    let mut data = Vec::with_capacity(size as usize / 4);
                    for _ in 0..(size / 4) {
                        data.push(file.read_f32()?);
                    }
                    group.movt = data;
                }
                "MOBA" => {
                    let mut data = Vec::with_capacity(size as usize / 2);
                    for _ in 0..(size / 2) {
                        data.push(file.read_u16()?);
                    }
                    group.moba = data;
                }
                "MODR" => {
                    let mut data = Vec::with_capacity(size as usize / 2);
                    for _ in 0..(size / 2) {
                        data.push(file.read_u16()?);
                    }
                    group.doodad_refs = data;
                }
                "MLIQ" => {
                    group.liquflags |= 1;
                    let header = WmoLiquidHeader {
                        xverts: file.read_i32()?,
                        yverts: file.read_i32()?,
                        xtiles: file.read_i32()?,
                        ytiles: file.read_i32()?,
                        pos_x: file.read_f32()?,
                        pos_y: file.read_f32()?,
                        pos_z: file.read_f32()?,
                        liquid_type: file.read_i16()?,
                    };
                    let vert_count = (header.xverts * header.yverts) as usize;
                    let mut verts = Vec::with_capacity(vert_count);
                    for _ in 0..vert_count {
                        let _unk1 = file.read_u16()?;
                        let _unk2 = file.read_u16()?;
                        let height = file.read_f32()?;
                        verts.push(WmoLiquidVert { _unk1, _unk2, height });
                    }
                    let byte_count = (header.xtiles * header.ytiles) as usize;
                    let bytes = file.read_vec(byte_count)?;
                    group.liquid_header = Some(header);
                    group.liquid_verts = verts;
                    group.liquid_bytes = bytes;
                }
                _ => {
                    if size > 0 {
                        file.seek(file.position() + size as usize);
                    }
                }
            }

            file.seek(nextpos);
        }

        Ok(Some(group))
    }

    fn should_skip(&self, root: &WmoRoot) -> bool {
        if (self.mogp_flags & 0x80) != 0 {
            return true;
        }
        if (self.mogp_flags & 0x4000000) != 0 {
            return true;
        }
        if self.group_name >= 0 && (self.group_name as usize) < root.group_names.len() {
            if let Some(name) = read_cstring(&root.group_names, self.group_name as usize) {
                if name == "antiportal" {
                    return true;
                }
            }
        }
        false
    }

    fn write_group(
        &mut self,
        out: &mut std::fs::File,
        root: &WmoRoot,
        precise: bool,
        filename: &str,
    ) -> anyhow::Result<u32> {
        out.write_u32::<LittleEndian>(self.mogp_flags as u32)?;
        out.write_u32::<LittleEndian>(self.group_wmo_id)?;
        for value in self.bbcorn1 {
            out.write_f32::<LittleEndian>(value)?;
        }
        for value in self.bbcorn2 {
            out.write_f32::<LittleEndian>(value)?;
        }
        out.write_u32::<LittleEndian>(self.liquflags)?;

        let mut n_col_triangles = 0u32;
        let moba_size = self.moba.len();
        let moba_batch = moba_size / 12;
        let mut moba_ex = Vec::with_capacity(moba_batch);
        let mut i = 8;
        while i < moba_size {
            moba_ex.push(self.moba[i] as u32);
            i += 12;
        }

        out.write_all(b"GRP ")?;
        let moba_size_grp = (moba_batch * 4 + 4) as u32;
        out.write_u32::<LittleEndian>(moba_size_grp)?;
        out.write_u32::<LittleEndian>(moba_batch as u32)?;
        for value in moba_ex {
            out.write_u32::<LittleEndian>(value)?;
        }

        if precise {
            let n_triangles = (self.mopy.len() / 2) as u32;
            let n_indexes = n_triangles * 3;
            out.write_all(b"INDX")?;
            let wsize = 4 + 2 * n_indexes;
            out.write_u32::<LittleEndian>(wsize)?;
            out.write_u32::<LittleEndian>(n_indexes)?;
            for idx in &self.movi {
                out.write_u16::<LittleEndian>(*idx)?;
            }

            out.write_all(b"VERT")?;
            let n_vertices = (self.movt.len() / 3) as u32;
            let wsize = 4 + 12 * n_vertices;
            out.write_u32::<LittleEndian>(wsize)?;
            out.write_u32::<LittleEndian>(n_vertices)?;
            for value in &self.movt {
                out.write_f32::<LittleEndian>(*value)?;
            }

            n_col_triangles = n_triangles;
        } else {
            let n_triangles = (self.mopy.len() / 2) as usize;
            let n_vertices = self.movt.len() / 3;
            let mut movi_ex = vec![0u16; n_triangles * 3];
            let mut index_renum = vec![-1i32; n_vertices];

            for tri in 0..n_triangles {
                let flag = self.mopy[2 * tri];
                let is_render_face = (flag & WMO_MATERIAL_RENDER) != 0 && (flag & WMO_MATERIAL_DETAIL) == 0;
                let is_collision = (flag & WMO_MATERIAL_COLLISION) != 0 || is_render_face;
                if !is_collision {
                    continue;
                }

                for j in 0..3 {
                    let idx = self.movi[3 * tri + j] as usize;
                    index_renum[idx] = 1;
                    movi_ex[3 * n_col_triangles as usize + j] = self.movi[3 * tri + j];
                }
                n_col_triangles += 1;
            }

            let mut n_col_vertices = 0i32;
            for value in &mut index_renum {
                if *value == 1 {
                    *value = n_col_vertices;
                    n_col_vertices += 1;
                }
            }

            for i in 0..(n_col_triangles as usize * 3) {
                let idx = movi_ex[i] as usize;
                movi_ex[i] = index_renum[idx] as u16;
            }

            out.write_all(b"INDX")?;
            let wsize = 4 + 2 * n_col_triangles * 3;
            out.write_u32::<LittleEndian>(wsize)?;
            out.write_u32::<LittleEndian>(n_col_triangles * 3)?;
            for i in 0..(n_col_triangles as usize * 3) {
                out.write_u16::<LittleEndian>(movi_ex[i])?;
            }

            out.write_all(b"VERT")?;
            let wsize = 4 + (n_col_vertices as u32) * 12;
            out.write_u32::<LittleEndian>(wsize)?;
            out.write_u32::<LittleEndian>(n_col_vertices as u32)?;

            for i in 0..n_vertices {
                if index_renum[i] >= 0 {
                    let base = i * 3;
                    out.write_f32::<LittleEndian>(self.movt[base])?;
                    out.write_f32::<LittleEndian>(self.movt[base + 1])?;
                    out.write_f32::<LittleEndian>(self.movt[base + 2])?;
                }
            }
        }

        if !self.liquid_verts.is_empty() {
            let Some(mut header) = self.liquid_header.take() else {
                return Ok(n_col_triangles);
            };

            out.write_all(b"LIQU")?;
            let header_size = 32u32;
            let size = header_size as usize + self.liquid_verts.len() * 8 + self.liquid_bytes.len();
            out.write_u32::<LittleEndian>(size as u32)?;

            let mut liquid_entry = if (root.flags & 4) != 0 {
                header.liquid_type as u32
            } else if header.liquid_type == 15 {
                0
            } else {
                header.liquid_type as u32 + 1
            };

            if liquid_entry == 0 {
                let mut found = None;
                for &b in &self.liquid_bytes {
                    if (b & 0xF) != 15 {
                        found = Some((b & 0xF) as u32 + 1);
                        break;
                    }
                }
                if let Some(value) = found {
                    liquid_entry = value;
                }
            }

            if liquid_entry != 0 && liquid_entry < 21 {
                match (liquid_entry - 1) & 3 {
                    0 => {
                        liquid_entry = if (self.mogp_flags & 0x80000) != 0 { 2 } else { 1 };
                        if liquid_entry == 1 && filename.contains("Coilfang_Raid") {
                            liquid_entry = 41;
                        }
                    }
                    1 => liquid_entry = 2,
                    2 => liquid_entry = 3,
                    3 => {
                        if filename.contains("Stratholme_raid") {
                            liquid_entry = 21;
                        } else {
                            liquid_entry = 4;
                        }
                    }
                    _ => {}
                }
            }

            header.liquid_type = liquid_entry as i16;

            out.write_i32::<LittleEndian>(header.xverts)?;
            out.write_i32::<LittleEndian>(header.yverts)?;
            out.write_i32::<LittleEndian>(header.xtiles)?;
            out.write_i32::<LittleEndian>(header.ytiles)?;
            out.write_f32::<LittleEndian>(header.pos_x)?;
            out.write_f32::<LittleEndian>(header.pos_y)?;
            out.write_f32::<LittleEndian>(header.pos_z)?;
            out.write_i16::<LittleEndian>(header.liquid_type)?;
            out.write_u16::<LittleEndian>(0)?; // padding to match C++ sizeof(WMOLiquidHeader)

            for vert in &self.liquid_verts {
                out.write_f32::<LittleEndian>(vert.height)?;
            }
            out.write_all(&self.liquid_bytes)?;
        }

        Ok(n_col_triangles)
    }
}

fn read_cstring(data: &[u8], offset: usize) -> Option<String> {
    if offset >= data.len() {
        return None;
    }
    let mut end = offset;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    Some(String::from_utf8_lossy(&data[offset..end]).to_string())
}
#[derive(Clone, Debug)]
struct MapEntry {
    id: u32,
    name: String,
}

fn read_map_dbc(context: &mut VmapContext) -> anyhow::Result<Vec<MapEntry>> {
    let dbc_bytes = context
        .mpq
        .open_file("DBFilesClient\\Map.dbc")
        .context("Map.dbc not found")?;

    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let mut entries = Vec::with_capacity(dbc.record_count());
    for idx in 0..dbc.record_count() {
        if let Some(record) = dbc.record(idx) {
            let id = record.get_u32(0).unwrap_or(0);
            let name = record.get_string(1).unwrap_or_default();
            entries.push(MapEntry { id, name });
        }
    }

    Ok(entries)
}

fn parse_maps(context: &mut VmapContext, maps: &[MapEntry]) -> anyhow::Result<()> {
    for map in maps {
        let wdt_name = format!("World\\Maps\\{}\\{}.wdt", map.name, map.name);
        let Some(wdt_bytes) = context.mpq.open_file(&wdt_name) else {
            continue;
        };

        let wdt = match read_wdt(&wdt_bytes) {
            Ok(value) => value,
            Err(err) => {
                if is_missing_mver(&err) {
                    tracing::warn!("Skipping map {} due to WDT parse error: {}", map.name, err);
                    continue;
                }
                return Err(err);
            }
        };
        parse_wdt_global_wmo(context, map, &wdt)?;

        for x in 0..64 {
            for y in 0..64 {
                let Some(tile) = wdt.get_tile(x, y) else {
                    continue;
                };
                if !tile.has_adt {
                    continue;
                }

                let adt_name = format!("World\\Maps\\{}\\{}_{}_{}.adt", map.name, map.name, x, y);
                let Some(adt_bytes) = context.mpq.open_file(&adt_name) else {
                    continue;
                };

                if let Err(err) = parse_adt_tile(context, map, x as u32, y as u32, &adt_bytes) {
                    if is_missing_mver(&err) {
                        tracing::warn!("Skipping ADT {} due to parse error: {}", adt_name, err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    Ok(())
}

fn read_wdt(data: &[u8]) -> anyhow::Result<wow_wdt::WdtFile> {
    let mut reader = WdtReader::new(Cursor::new(data), WowVersion::TBC);
    reader.read().map_err(|err| anyhow::anyhow!(err))
}

fn is_missing_mver(err: &anyhow::Error) -> bool {
    err.to_string().contains("Missing required chunk: MVER")
}

fn parse_wdt_global_wmo(
    context: &mut VmapContext,
    map: &MapEntry,
    wdt: &wow_wdt::WdtFile,
) -> anyhow::Result<()> {
    let mut wmo_names = Vec::new();

    if let Some(mwmo) = &wdt.mwmo {
        for name in &mwmo.filenames {
            let fixed = normalize_filename(name);
            wmo_names.push(fixed);
        }
    }

    if let Some(modf) = &wdt.modf {
        let mut dirfile = open_dir_bin(&context.buildings_dir)?;
        for entry in &modf.entries {
            let Some(name) = wmo_names.get(entry.id as usize) else {
                continue;
            };

            let inst = WmoInstanceData {
                unique_id: entry.unique_id,
                position: Vec3::new(entry.position[0], entry.position[1], entry.position[2]),
                rotation: Vec3::new(entry.rotation[0], entry.rotation[1], entry.rotation[2]),
                bounds: AaBox {
                    min: Vec3::new(entry.lower_bounds[0], entry.lower_bounds[1], entry.lower_bounds[2]),
                    max: Vec3::new(entry.upper_bounds[0], entry.upper_bounds[1], entry.upper_bounds[2]),
                },
                flags: entry.flags,
                doodad_set: entry.doodad_set,
                name_set: entry.name_set,
            };

            write_wmo_instance(context, &mut dirfile, map, 65, 65, name, &inst)?;
            if let Some(doodads) = context.wmo_doodads.get(name).cloned() {
                extract_doodad_set(context, &mut dirfile, map, 65, 65, &doodads, &inst)?;
            }
        }
    }

    Ok(())
}

fn parse_adt_tile(
    context: &mut VmapContext,
    map: &MapEntry,
    tile_x: u32,
    tile_y: u32,
    adt_bytes: &[u8],
) -> anyhow::Result<()> {
    let mut cursor = Cursor::new(adt_bytes);
    let parsed = parse_adt(&mut cursor)?;

    let root = match parsed {
        ParsedAdt::Root(root) => root,
        _ => return Ok(()),
    };

    let mut model_names = Vec::new();
    for model in &root.models {
        let mut bytes = model.as_bytes().to_vec();
        fixnamen(&mut bytes);
        let fixed_path = String::from_utf8_lossy(&bytes).to_string();
        let fixed_name = normalize_filename(&fixed_path);
        if let Some(name) = extract_single_model(context, &fixed_path)? {
            model_names.push(name);
        } else {
            model_names.push(fixed_name);
        }
    }

    let mut wmo_names = Vec::new();
    for wmo in &root.wmos {
        let fixed = normalize_filename(wmo);
        wmo_names.push(fixed);
    }

    let mut dirfile = open_dir_bin(&context.buildings_dir)?;

    for placement in &root.doodad_placements {
        let Some(name) = model_names.get(placement.name_id as usize) else {
            continue;
        };
        let inst = ModelInstanceData {
            id: placement.name_id,
            position: Vec3::new(placement.position[0], placement.position[1], placement.position[2]),
            rotation: Vec3::new(placement.rotation[0], placement.rotation[1], placement.rotation[2]),
            scale: placement.scale,
        };
        write_model_instance(context, &mut dirfile, map, tile_x, tile_y, name, &inst)?;
    }

    for placement in &root.wmo_placements {
        let Some(name) = wmo_names.get(placement.name_id as usize) else {
            continue;
        };

        let inst = WmoInstanceData {
            unique_id: placement.unique_id,
            position: Vec3::new(placement.position[0], placement.position[1], placement.position[2]),
            rotation: Vec3::new(placement.rotation[0], placement.rotation[1], placement.rotation[2]),
            bounds: AaBox {
                min: Vec3::new(placement.extents_min[0], placement.extents_min[1], placement.extents_min[2]),
                max: Vec3::new(placement.extents_max[0], placement.extents_max[1], placement.extents_max[2]),
            },
            flags: placement.flags,
            doodad_set: placement.doodad_set,
            name_set: placement.name_set,
        };

        write_wmo_instance(context, &mut dirfile, map, tile_x, tile_y, name, &inst)?;
        if let Some(doodads) = context.wmo_doodads.get(name).cloned() {
            extract_doodad_set(context, &mut dirfile, map, tile_x, tile_y, &doodads, &inst)?;
        }
    }

    Ok(())
}

fn open_dir_bin(buildings_dir: &Path) -> anyhow::Result<std::fs::File> {
    let path = buildings_dir.join(DIR_BIN);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    Ok(file)
}

#[derive(Clone, Copy, Debug)]
struct ModelInstanceData {
    id: u32,
    position: Vec3,
    rotation: Vec3,
    scale: u16,
}

#[derive(Clone, Copy, Debug)]
struct WmoInstanceData {
    unique_id: u32,
    position: Vec3,
    rotation: Vec3,
    bounds: AaBox,
    flags: u16,
    doodad_set: u16,
    name_set: u16,
}

fn write_model_instance(
    context: &mut VmapContext,
    dirfile: &mut std::fs::File,
    map: &MapEntry,
    tile_x: u32,
    tile_y: u32,
    name: &str,
    inst: &ModelInstanceData,
) -> anyhow::Result<()> {
    let output = context.buildings_dir.join(name);
    if !output.exists() {
        return Ok(());
    }

    let n_vertices = read_model_vertex_count(&output)?;
    if n_vertices == 0 {
        return Ok(());
    }

    let pos = fix_coords(inst.position);
    let rot = inst.rotation;
    let scale = inst.scale as f32 / 1024.0;

    let mut flags = MOD_M2;
    if tile_x == 65 && tile_y == 65 {
        flags |= MOD_WORLDSPAWN;
    }

    let unique_id = context.unique_ids.generate(inst.id, 0);
    let name_bytes = name.as_bytes();

    dirfile.write_u32::<LittleEndian>(map.id)?;
    dirfile.write_u32::<LittleEndian>(tile_x)?;
    dirfile.write_u32::<LittleEndian>(tile_y)?;
    dirfile.write_u32::<LittleEndian>(flags)?;
    dirfile.write_u16::<LittleEndian>(0)?;
    dirfile.write_u32::<LittleEndian>(unique_id)?;
    write_vec3(dirfile, pos)?;
    write_vec3(dirfile, rot)?;
    dirfile.write_f32::<LittleEndian>(scale)?;
    dirfile.write_u32::<LittleEndian>(name_bytes.len() as u32)?;
    dirfile.write_all(name_bytes)?;

    Ok(())
}

fn write_wmo_instance(
    context: &mut VmapContext,
    dirfile: &mut std::fs::File,
    map: &MapEntry,
    tile_x: u32,
    tile_y: u32,
    name: &str,
    inst: &WmoInstanceData,
) -> anyhow::Result<()> {
    if (inst.flags & 0x1) != 0 {
        return Ok(());
    }

    let output = context.buildings_dir.join(name);
    if !output.exists() {
        return Ok(());
    }

    let n_vertices = read_model_vertex_count(&output)?;
    if n_vertices == 0 {
        return Ok(());
    }

    let mut position = inst.position;
    if position.x == 0.0 && position.z == 0.0 {
        position.x = 533.33333 * 32.0;
        position.z = 533.33333 * 32.0;
    }

    let position = fix_coords(position);
    let bounds = AaBox {
        min: fix_coords(inst.bounds.min),
        max: fix_coords(inst.bounds.max),
    };

    let mut flags = MOD_HAS_BOUND;
    if tile_x == 65 && tile_y == 65 {
        flags |= MOD_WORLDSPAWN;
    }

    let unique_id = context.unique_ids.generate(inst.unique_id, 0);
    let name_bytes = name.as_bytes();
    let scale = 1.0f32;

    dirfile.write_u32::<LittleEndian>(map.id)?;
    dirfile.write_u32::<LittleEndian>(tile_x)?;
    dirfile.write_u32::<LittleEndian>(tile_y)?;
    dirfile.write_u32::<LittleEndian>(flags)?;
    dirfile.write_u16::<LittleEndian>(inst.name_set)?;
    dirfile.write_u32::<LittleEndian>(unique_id)?;
    write_vec3(dirfile, position)?;
    write_vec3(dirfile, inst.rotation)?;
    dirfile.write_f32::<LittleEndian>(scale)?;
    write_aabox(dirfile, bounds)?;
    dirfile.write_u32::<LittleEndian>(name_bytes.len() as u32)?;
    dirfile.write_all(name_bytes)?;

    Ok(())
}

fn write_vec3(out: &mut std::fs::File, v: Vec3) -> anyhow::Result<()> {
    out.write_f32::<LittleEndian>(v.x)?;
    out.write_f32::<LittleEndian>(v.y)?;
    out.write_f32::<LittleEndian>(v.z)?;
    Ok(())
}

fn write_aabox(out: &mut std::fs::File, b: AaBox) -> anyhow::Result<()> {
    write_vec3(out, b.min)?;
    write_vec3(out, b.max)?;
    Ok(())
}

fn read_model_vertex_count(path: &Path) -> anyhow::Result<u32> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 12];
    file.read_exact(&mut buf)?;
    let mut cursor = Cursor::new(&buf[8..12]);
    Ok(cursor.read_u32::<LittleEndian>()?)
}
fn extract_doodad_set(
    context: &mut VmapContext,
    dirfile: &mut std::fs::File,
    map: &MapEntry,
    tile_x: u32,
    tile_y: u32,
    doodad_data: &WmoDoodadData,
    wmo: &WmoInstanceData,
) -> anyhow::Result<()> {
    if (wmo.doodad_set as usize) >= doodad_data.sets.len() {
        return Ok(());
    }

    let set = &doodad_data.sets[wmo.doodad_set as usize];
    let wmo_position = fix_coords(wmo.position);
    let wmo_rot = matrix_from_euler_zyx(
        deg_to_rad(wmo.rotation.y),
        deg_to_rad(wmo.rotation.x),
        deg_to_rad(wmo.rotation.z),
    );

    let mut doodad_id: u16 = 0;
    for reference in &doodad_data.references {
        let ref_idx = *reference as u32;
        if ref_idx < set.start_index || ref_idx >= set.start_index + set.count {
            continue;
        }

        let spawn = &doodad_data.spawns[*reference as usize];
        let Some(path) = read_cstring(&doodad_data.paths_blob, spawn.name_index as usize) else {
            continue;
        };

        let mut model_name = get_plain_name(&path).to_string();
        let mut name_bytes = model_name.as_bytes().to_vec();
        fixnamen(&mut name_bytes);
        fixname2(&mut name_bytes);
        model_name = String::from_utf8_lossy(&name_bytes).to_string();

        if model_name.len() > 3 {
            let ext = model_name[model_name.len() - 4..].to_string();
            if ext.eq_ignore_ascii_case(".mdx") || ext.eq_ignore_ascii_case(".mdl") {
                model_name.truncate(model_name.len() - 2);
                model_name.push('2');
            }
        }

        let model_path = context.buildings_dir.join(&model_name);
        if !model_path.exists() {
            continue;
        }

        let n_vertices = read_model_vertex_count(&model_path)?;
        if n_vertices == 0 {
            continue;
        }

        doodad_id = doodad_id.wrapping_add(1);

        let doodad_pos = Vec3::new(spawn.position.x, spawn.position.y, spawn.position.z);
        let position = add_vec3(wmo_position, mat3_mul_vec3(wmo_rot, doodad_pos));

        let quat = Quaternion {
            x: spawn.rotation.x,
            y: spawn.rotation.y,
            z: spawn.rotation.z,
            w: spawn.rotation.w,
        };
        let rot_matrix = quat_to_matrix(quat);
        let combined = matrix_mul(rot_matrix, wmo_rot);
        let (rx, ry, rz) = matrix_to_euler_xyz(combined);

        let rotation = Vec3::new(rad_to_deg(ry), rad_to_deg(rz), rad_to_deg(rx));

        let mut flags = MOD_M2;
        if tile_x == 65 && tile_y == 65 {
            flags |= MOD_WORLDSPAWN;
        }

        let unique_id = context.unique_ids.generate(wmo.unique_id, doodad_id);
        let name_bytes = model_name.as_bytes();

        dirfile.write_u32::<LittleEndian>(map.id)?;
        dirfile.write_u32::<LittleEndian>(tile_x)?;
        dirfile.write_u32::<LittleEndian>(tile_y)?;
        dirfile.write_u32::<LittleEndian>(flags)?;
        dirfile.write_u16::<LittleEndian>(0)?;
        dirfile.write_u32::<LittleEndian>(unique_id)?;
        write_vec3(dirfile, position)?;
        write_vec3(dirfile, rotation)?;
        dirfile.write_f32::<LittleEndian>(spawn.scale)?;
        dirfile.write_u32::<LittleEndian>(name_bytes.len() as u32)?;
        dirfile.write_all(name_bytes)?;
    }

    Ok(())
}

fn add_vec3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn mat3_mul_vec3(m: [[f32; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}

fn extract_gameobject_models(context: &mut VmapContext) -> anyhow::Result<()> {
    let dbc_bytes = context
        .mpq
        .open_file("DBFilesClient\\GameObjectDisplayInfo.dbc")
        .context("GameObjectDisplayInfo.dbc not found")?;
    let dbc = DbcFile::from_bytes(&dbc_bytes)?;
    dbc.validate()?;

    let list_path = context.buildings_dir.join(TEMP_GAMEOBJECT_LIST);
    let mut list_file = std::fs::File::create(&list_path)?;

    for idx in 0..dbc.record_count() {
        let Some(record) = dbc.record(idx) else {
            continue;
        };

        let path = record.get_string(1).unwrap_or_default();
        if path.len() < 4 {
            continue;
        }

        let mut bytes = path.as_bytes().to_vec();
        fixnamen(&mut bytes);
        let fixed_path = String::from_utf8_lossy(&bytes).to_string();
        let mut name = get_plain_name(&fixed_path).to_string();
        let mut name_bytes = name.as_bytes().to_vec();
        fixname2(&mut name_bytes);
        name = String::from_utf8_lossy(&name_bytes).to_string();

        let ext = get_extension(&name).unwrap_or("");

        let mut result = false;
        if ext.eq_ignore_ascii_case("wmo") {
            let mut temp = fixed_path.clone();
            result = extract_single_wmo(context, &mut temp)?;
        } else if ext.eq_ignore_ascii_case("mdl") {
            continue;
        } else {
            if let Some(converted) = extract_single_model(context, &fixed_path)? {
                name = converted;
                result = true;
            }
        }

        if result {
            let display_id = record.get_u32(0).unwrap_or(0);
            list_file.write_u32::<LittleEndian>(display_id)?;
            list_file.write_u32::<LittleEndian>(name.len() as u32)?;
            list_file.write_all(name.as_bytes())?;
        }
    }

    Ok(())
}
