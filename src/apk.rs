use crate::bytes::Result;
use std::{fs::File, io::Read, path::Path};

/// Refuse entries bigger than this when decompressing (zip-bomb guard).
const MAX_ENTRY: u64 = 512 << 20;

pub struct Apk {
    zip: zip::ZipArchive<File>,
}

impl Apk {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let zip = zip::ZipArchive::new(f).map_err(|e| format!("{}: not an APK/zip: {e}", path.display()))?;
        Ok(Self { zip })
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let entry = self.zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
        if entry.size() > MAX_ENTRY {
            return Err(format!("{name}: {} bytes uncompressed, over the {MAX_ENTRY} limit", entry.size()));
        }
        let mut out = Vec::with_capacity(entry.size() as usize);
        // take() also caps entries whose header lies about their size
        entry.take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|e| format!("{name}: {e}"))?;
        if out.len() as u64 > MAX_ENTRY {
            return Err(format!("{name}: over the {MAX_ENTRY} byte limit"));
        }
        Ok(out)
    }

    pub fn has(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// classes.dex, classes2.dex, ... in load order.
    pub fn dex_names(&self) -> Vec<String> {
        let mut names: Vec<(u32, String)> = self
            .zip
            .file_names()
            .filter_map(|n| {
                let n = n.ok()?;
                let num = n.strip_prefix("classes")?.strip_suffix(".dex")?;
                let order = if num.is_empty() { 1 } else { num.parse().ok().filter(|&n| n >= 2)? };
                Some((order, n.to_string()))
            })
            .collect();
        names.sort();
        names.into_iter().map(|(_, n)| n).collect()
    }
}
