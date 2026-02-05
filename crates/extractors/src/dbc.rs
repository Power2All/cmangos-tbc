use std::io::{Cursor, Read};

pub struct DbcFile {
    record_count: u32,
    field_count: u32,
    record_size: u32,
    data: Vec<u8>,
    string_table: Vec<u8>,
}

pub struct DbcRecord<'a> {
    file: &'a DbcFile,
    index: usize,
}

impl DbcFile {
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic)?;
        if &magic != b"WDBC" {
            anyhow::bail!("Invalid DBC magic");
        }

        let record_count = read_u32(&mut cursor)?;
        let field_count = read_u32(&mut cursor)?;
        let record_size = read_u32(&mut cursor)?;
        let string_size = read_u32(&mut cursor)?;

        let data_size = record_count
            .checked_mul(record_size)
            .ok_or_else(|| anyhow::anyhow!("DBC data size overflow"))? as usize;
        let mut data = vec![0u8; data_size];
        cursor.read_exact(&mut data)?;

        let mut string_table = vec![0u8; string_size as usize];
        cursor.read_exact(&mut string_table)?;

        Ok(Self {
            record_count,
            field_count,
            record_size,
            data,
            string_table,
        })
    }

    pub fn record_count(&self) -> usize {
        self.record_count as usize
    }

    pub fn record(&self, index: usize) -> Option<DbcRecord<'_>> {
        if index >= self.record_count() {
            return None;
        }
        Some(DbcRecord { file: self, index })
    }

    pub fn max_id(&self) -> u32 {
        let mut max_id = 0u32;
        for idx in 0..self.record_count() {
            if let Some(record) = self.record(idx) {
                let id = record.get_u32(0).unwrap_or(0);
                if id > max_id {
                    max_id = id;
                }
            }
        }
        max_id
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.field_count * 4 != self.record_size {
            anyhow::bail!("DBC header mismatch: field_count * 4 != record_size");
        }
        Ok(())
    }
}

impl<'a> DbcRecord<'a> {
    pub fn get_u32(&self, field: usize) -> Option<u32> {
        let offset = field.checked_mul(4)?;
        let start = self.index * self.file.record_size as usize + offset;
        let end = start + 4;
        if end > self.file.data.len() {
            return None;
        }
        Some(u32::from_le_bytes(
            self.file.data[start..end].try_into().ok()?,
        ))
    }

    pub fn get_string(&self, field: usize) -> Option<String> {
        let offset = self.get_u32(field)? as usize;
        if offset >= self.file.string_table.len() {
            return Some(String::new());
        }
        let slice = &self.file.string_table[offset..];
        let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        let bytes = &slice[..len];
        Some(String::from_utf8_lossy(bytes).to_string())
    }
}

fn read_u32<R: Read>(reader: &mut R) -> anyhow::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}
