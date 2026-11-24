//! resources.arsc: resource id (0xPPTTEEEE) → value.

use crate::{
    axml::{self, Value},
    bytes::*,
};
use std::collections::HashMap;

const RES_TABLE: u16 = 0x0002;
const RES_STRING_POOL: u16 = 0x0001;
const RES_TABLE_PACKAGE: u16 = 0x0200;
const RES_TABLE_TYPE: u16 = 0x0201;
const NO_ENTRY: u32 = 0xffff_ffff;
const FLAG_SPARSE: u8 = 0x01;
const FLAG_OFFSET16: u8 = 0x02;
const ENTRY_COMPLEX: u16 = 0x0001;
const ENTRY_COMPACT: u16 = 0x0008;

/// shortcut: keeps one value per id, the default configuration's when there is one; pick by
/// locale/density/night mode once the loader has a device configuration.
pub struct Resources {
    values: HashMap<u32, Value>,
}

impl Resources {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if u16_at(b, 0)? != RES_TABLE {
            return Err("not a resources.arsc table".into());
        }
        let end = (u32_at(b, 4)? as usize).min(b.len());
        let mut strings = vec![];
        let mut values = HashMap::new();
        let mut off = u16_at(b, 2)? as usize;
        while off + 8 <= end {
            let size = u32_at(b, off + 4)? as usize;
            if size < 8 {
                return Err(format!("bad chunk size {size} at 0x{off:x}"));
            }
            let chunk = slice(b, off, size)?;
            match u16_at(chunk, 0)? {
                RES_STRING_POOL => strings = axml::string_pool(chunk)?,
                RES_TABLE_PACKAGE => package(chunk, &strings, &mut values)?,
                _ => {}
            }
            off += size;
        }
        Ok(Resources { values })
    }

    /// The value for `id`, following @ref chains to the final value.
    pub fn get(&self, id: u32) -> Option<&Value> {
        let mut v = self.values.get(&id)?;
        for _ in 0..16 {
            match v {
                Value::Ref(r) => v = self.values.get(r)?,
                _ => return Some(v),
            }
        }
        None // reference cycle
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

fn package(c: &[u8], strings: &[String], values: &mut HashMap<u32, Value>) -> Result<()> {
    let pkg = u32_at(c, 8)? & 0xff;
    let mut off = u16_at(c, 2)? as usize;
    while off + 8 <= c.len() {
        let size = u32_at(c, off + 4)? as usize;
        if size < 8 {
            return Err(format!("bad package chunk size {size} at 0x{off:x}"));
        }
        let chunk = slice(c, off, size)?;
        if u16_at(chunk, 0)? == RES_TABLE_TYPE {
            type_chunk(chunk, pkg, strings, values)?;
        }
        off += size; // type/key string pools and typeSpec chunks aren't needed for lookups
    }
    Ok(())
}

fn type_chunk(c: &[u8], pkg: u32, strings: &[String], values: &mut HashMap<u32, Value>) -> Result<()> {
    let header = u16_at(c, 2)? as usize;
    let type_id = u8_at(c, 8)? as u32;
    let flags = u8_at(c, 9)?;
    let count = u32_at(c, 12)? as usize;
    let entries = u32_at(c, 16)? as usize;
    let config_size = u32_at(c, 20)? as usize;
    let is_default = slice(c, 24, config_size.saturating_sub(4))?.iter().all(|&b| b == 0);

    for i in 0..count {
        // (entry index, byte offset from `entries`) in whichever of the three index encodings is used
        let (idx, off) = if flags & FLAG_SPARSE != 0 {
            (u16_at(c, header + i * 4)? as usize, u16_at(c, header + i * 4 + 2)? as usize * 4)
        } else if flags & FLAG_OFFSET16 != 0 {
            match u16_at(c, header + i * 2)? {
                0xffff => continue,
                o => (i, o as usize * 4),
            }
        } else {
            match u32_at(c, header + i * 4)? {
                NO_ENTRY => continue,
                o => (i, o as usize),
            }
        };
        let e = entries + off;
        let eflags = u16_at(c, e + 2)?;
        let (ty, data) = if eflags & ENTRY_COMPACT != 0 {
            ((eflags >> 8) as u8, u32_at(c, e + 4)?)
        } else if eflags & ENTRY_COMPLEX != 0 {
            continue; // shortcut: bag entries (styles, arrays, plurals) skipped until themes/arrays are needed
        } else {
            let v = e + u16_at(c, e)? as usize; // Res_value follows the entry header
            (u8_at(c, v + 3)?, u32_at(c, v + 4)?)
        };
        let id = pkg << 24 | type_id << 16 | idx as u32;
        if is_default || !values.contains_key(&id) {
            values.insert(id, axml::res_value(ty, data, strings));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::axml::Value;

    #[test]
    fn dimensions_decode() {
        assert_eq!(Value::Other { ty: 0x05, data: 0x1801 }.dimension(), Some((24.0, 1))); // 24dp
        assert_eq!(Value::Other { ty: 0x05, data: 0x1402 }.dimension(), Some((20.0, 2))); // 20sp
        assert_eq!(Value::Other { ty: 0x05, data: 0xffff_e800 }.dimension(), Some((-24.0, 0))); // -24px
        assert_eq!(Value::Int(5).dimension(), None);
    }
}
