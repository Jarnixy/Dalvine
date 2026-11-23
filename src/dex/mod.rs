//! classes.dex: id tables, class definitions, method bodies (raw bytecode, not yet decoded).

use crate::bytes::*;

const NO_INDEX: u32 = 0xffff_ffff;

pub struct Dex {
    pub strings: Vec<String>,
    /// type_ids: index into `strings` (the descriptor, e.g. `Ljava/lang/String;`)
    pub types: Vec<u32>,
    pub protos: Vec<Proto>,
    pub fields: Vec<FieldId>,
    pub methods: Vec<MethodId>,
    pub classes: Vec<ClassDef>,
}

pub struct Proto {
    pub shorty: u32,
    pub ret: u32,
    pub params: Vec<u16>,
}

pub struct FieldId {
    pub class: u16,
    pub ty: u16,
    pub name: u32,
}

pub struct MethodId {
    pub class: u16,
    pub proto: u16,
    pub name: u32,
}

pub struct ClassDef {
    pub class: u32,
    pub access: u32,
    pub superclass: Option<u32>,
    pub interfaces: Vec<u16>,
    pub static_fields: Vec<EncodedField>,
    pub instance_fields: Vec<EncodedField>,
    pub direct_methods: Vec<EncodedMethod>,
    pub virtual_methods: Vec<EncodedMethod>,
}

pub struct EncodedField {
    pub field: u32,
    pub access: u32,
}

pub struct EncodedMethod {
    pub method: u32,
    pub access: u32,
    /// None for abstract/native methods
    pub code: Option<Code>,
}

pub struct Code {
    pub registers: u16,
    pub ins: u16,
    pub outs: u16,
    pub tries: u16,
    /// bytecode in 16-bit code units
    pub insns: Vec<u16>,
}

impl Dex {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if slice(b, 0, 4)? != b"dex\n" {
            return Err("not a DEX file (bad magic)".into());
        }
        if u32_at(b, 0x28)? != 0x1234_5678 {
            return Err("unsupported DEX endianness".into());
        }
        let strings = table(b, 0x38, 4)?
            .map(|o| mutf8(b, u32_at(b, o)? as usize))
            .collect::<Result<_>>()?;
        let types = table(b, 0x40, 4)?.map(|o| u32_at(b, o)).collect::<Result<_>>()?;
        let protos = table(b, 0x48, 12)?
            .map(|o| {
                Ok(Proto { shorty: u32_at(b, o)?, ret: u32_at(b, o + 4)?, params: type_list(b, u32_at(b, o + 8)?)? })
            })
            .collect::<Result<_>>()?;
        let fields = table(b, 0x50, 8)?
            .map(|o| Ok(FieldId { class: u16_at(b, o)?, ty: u16_at(b, o + 2)?, name: u32_at(b, o + 4)? }))
            .collect::<Result<_>>()?;
        let methods = table(b, 0x58, 8)?
            .map(|o| Ok(MethodId { class: u16_at(b, o)?, proto: u16_at(b, o + 2)?, name: u32_at(b, o + 4)? }))
            .collect::<Result<_>>()?;
        let classes = table(b, 0x60, 32)?.map(|o| class_def(b, o)).collect::<Result<_>>()?;
        Ok(Dex { strings, types, protos, fields, methods, classes })
    }

    pub fn string(&self, i: u32) -> &str {
        self.strings.get(i as usize).map_or("<bad string>", |s| s)
    }

    pub fn type_name(&self, i: u32) -> &str {
        self.types.get(i as usize).map_or("<bad type>", |&s| self.string(s))
    }

    /// e.g. `(Landroid/os/Bundle;)V`, matching dexdump's "type" line.
    pub fn proto_sig(&self, i: u16) -> String {
        let Some(p) = self.protos.get(i as usize) else { return "<bad proto>".into() };
        let params: String = p.params.iter().map(|&t| self.type_name(t as u32)).collect();
        format!("({params}){}", self.type_name(p.ret))
    }

    /// e.g. `Lcom/x/Main;->onCreate(Landroid/os/Bundle;)V`, the key the runtime will dispatch on.
    pub fn method_sig(&self, i: u32) -> String {
        let Some(m) = self.methods.get(i as usize) else { return "<bad method>".into() };
        format!("{}->{}", self.type_name(m.class as u32), self.name_proto(i))
    }

    /// e.g. `onCreate(Landroid/os/Bundle;)V`: what overriding methods have in common.
    pub fn name_proto(&self, i: u32) -> String {
        let Some(m) = self.methods.get(i as usize) else { return "<bad method>".into() };
        format!("{}{}", self.string(m.name), self.proto_sig(m.proto))
    }

    /// e.g. `Lcom/x/Main;->count:I`; same string in every dex, so usable as a heap key.
    pub fn field_key(&self, i: u32) -> String {
        let Some(f) = self.fields.get(i as usize) else { return "<bad field>".into() };
        format!("{}->{}:{}", self.type_name(f.class as u32), self.string(f.name), self.type_name(f.ty as u32))
    }
}

/// Iterator over item offsets of an id table whose (size, offset) header pair is at `hdr`.
fn table(b: &[u8], hdr: usize, item: usize) -> Result<impl Iterator<Item = usize>> {
    let count = u32_at(b, hdr)? as usize;
    let off = u32_at(b, hdr + 4)? as usize;
    // validate the whole table up front, so a huge count can't drive a huge allocation
    slice(b, off, count.checked_mul(item).ok_or("table size overflow")?)?;
    Ok((0..count).map(move |i| off + i * item))
}

fn type_list(b: &[u8], off: u32) -> Result<Vec<u16>> {
    if off == 0 {
        return Ok(vec![]);
    }
    let off = off as usize;
    let n = u32_at(b, off)? as usize;
    Ok(slice(b, off + 4, n.checked_mul(2).ok_or("type_list overflow")?)?
        .chunks_exact(2)
        .map(|u| u16::from_le_bytes([u[0], u[1]]))
        .collect())
}

fn class_def(b: &[u8], o: usize) -> Result<ClassDef> {
    let superclass = u32_at(b, o + 8)?;
    let mut c = ClassDef {
        class: u32_at(b, o)?,
        access: u32_at(b, o + 4)?,
        superclass: (superclass != NO_INDEX).then_some(superclass),
        interfaces: type_list(b, u32_at(b, o + 12)?)?,
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
    };
    let data = u32_at(b, o + 24)? as usize;
    if data == 0 {
        return Ok(c); // marker interface / empty class
    }
    let mut p = data;
    let (sf, inf) = (uleb128(b, &mut p)?, uleb128(b, &mut p)?);
    let (dm, vm) = (uleb128(b, &mut p)?, uleb128(b, &mut p)?);
    c.static_fields = fields(b, &mut p, sf)?;
    c.instance_fields = fields(b, &mut p, inf)?;
    c.direct_methods = methods(b, &mut p, dm)?;
    c.virtual_methods = methods(b, &mut p, vm)?;
    Ok(c)
}

// indices are delta-encoded; the running total restarts for each list
fn fields(b: &[u8], p: &mut usize, n: u32) -> Result<Vec<EncodedField>> {
    let mut idx = 0u32;
    (0..n)
        .map(|_| {
            idx = idx.wrapping_add(uleb128(b, p)?);
            Ok(EncodedField { field: idx, access: uleb128(b, p)? })
        })
        .collect()
}

fn methods(b: &[u8], p: &mut usize, n: u32) -> Result<Vec<EncodedMethod>> {
    let mut idx = 0u32;
    (0..n)
        .map(|_| {
            idx = idx.wrapping_add(uleb128(b, p)?);
            let access = uleb128(b, p)?;
            let code_off = uleb128(b, p)? as usize;
            let code = if code_off == 0 { None } else { Some(code(b, code_off)?) };
            Ok(EncodedMethod { method: idx, access, code })
        })
        .collect()
}

fn code(b: &[u8], o: usize) -> Result<Code> {
    let n = u32_at(b, o + 12)? as usize;
    let insns = slice(b, o + 16, n.checked_mul(2).ok_or("insns overflow")?)?
        .chunks_exact(2)
        .map(|u| u16::from_le_bytes([u[0], u[1]]))
        .collect();
    Ok(Code { registers: u16_at(b, o)?, ins: u16_at(b, o + 2)?, outs: u16_at(b, o + 4)?, tries: u16_at(b, o + 6)?, insns })
}

/// string_data_item: uleb128 UTF-16 length, then Modified UTF-8 (CESU-8 surrogates, 0xC0 0x80 for NUL).
fn mutf8(b: &[u8], mut p: usize) -> Result<String> {
    uleb128(b, &mut p)?;
    let mut units = Vec::new();
    let next = |p: &mut usize| -> Result<u16> {
        let v = u8_at(b, *p)?;
        *p += 1;
        Ok(v as u16)
    };
    loop {
        let a = next(&mut p)?;
        let unit = match a {
            0 => break,
            0x01..=0x7f => a,
            0xc0..=0xdf => (a & 0x1f) << 6 | (next(&mut p)? & 0x3f),
            0xe0..=0xef => (a & 0x0f) << 12 | (next(&mut p)? & 0x3f) << 6 | (next(&mut p)? & 0x3f),
            _ => return Err(format!("bad MUTF-8 byte 0x{a:02x} before 0x{p:x}")),
        };
        units.push(unit);
    }
    Ok(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use super::mutf8;

    #[test]
    fn modified_utf8() {
        // len, "a", NUL as C0 80, "é", U+1F600 as a CESU-8 surrogate pair, terminator
        let b = [5, b'a', 0xc0, 0x80, 0xc3, 0xa9, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80, 0];
        assert_eq!(mutf8(&b, 0).unwrap(), "a\0é😀");
        assert!(mutf8(&[1, b'a'], 0).is_err()); // missing terminator
    }
}
