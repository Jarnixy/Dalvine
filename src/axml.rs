//! Android binary XML (AndroidManifest.xml and compiled res/layout/*.xml).

use crate::bytes::*;

const RES_STRING_POOL: u16 = 0x0001;
const RES_XML: u16 = 0x0003;
const RES_XML_START_ELEMENT: u16 = 0x0102;
const RES_XML_END_ELEMENT: u16 = 0x0103;
const RES_XML_RESOURCE_MAP: u16 = 0x0180;
const NO_INDEX: u32 = 0xffff_ffff;
const UTF8_FLAG: u32 = 1 << 8;
/// android:name, used when an obfuscator has blanked the attribute name strings.
const ATTR_ANDROID_NAME: u32 = 0x0101_0003;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i32),
    Bool(bool),
    /// @0x7f...... resource reference
    Ref(u32),
    Other { ty: u8, data: u32 },
}

/// Decodes a Res_value (shared by binary XML attributes and resources.arsc entries).
pub fn res_value(ty: u8, data: u32, strings: &[String]) -> Value {
    match ty {
        0x03 => Value::Str(strings.get(data as usize).cloned().unwrap_or_default()),
        0x10 | 0x11 => Value::Int(data as i32),
        0x12 => Value::Bool(data != 0),
        0x01 => Value::Ref(data),
        _ => Value::Other { ty, data },
    }
}

impl Value {
    /// TYPE_DIMENSION → (amount, unit) where unit 0=px 1=dp 2=sp 3=pt 4=in 5=mm.
    pub fn dimension(&self) -> Option<(f32, u8)> {
        const RADIX: [f32; 4] = [1.0 / 256.0, 1.0 / 32768.0, 1.0 / 8388608.0, 1.0 / 2147483648.0];
        match *self {
            Value::Other { ty: 0x05, data } => {
                Some(((data & 0xffff_ff00) as i32 as f32 * RADIX[(data >> 4 & 3) as usize], (data & 0xf) as u8))
            }
            _ => None,
        }
    }

    /// Any of the four compiled color types (already expanded to ARGB), or a plain int.
    pub fn color(&self) -> Option<u32> {
        match *self {
            Value::Other { ty: 0x1c..=0x1f, data } => Some(data),
            Value::Int(i) => Some(i as u32),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Attr {
    pub ns: Option<String>,
    pub name: String,
    pub res_id: Option<u32>,
    pub value: Value,
}

#[derive(Debug, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&Value> {
        self.attrs
            .iter()
            .find(|a| a.name == name || (name == "name" && a.name.is_empty() && a.res_id == Some(ATTR_ANDROID_NAME)))
            .map(|a| &a.value)
    }

    pub fn str_attr(&self, name: &str) -> Option<&str> {
        match self.attr(name)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Pre-order walk, `f(depth, element)`.
    pub fn walk(&self, depth: usize, f: &mut impl FnMut(usize, &Element)) {
        f(depth, self);
        for c in &self.children {
            c.walk(depth + 1, f);
        }
    }

    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.walk(0, &mut |d, e| {
            out += &format!("{:w$}E: {}\n", "", e.name, w = d * 2);
            for a in &e.attrs {
                let ns = if a.ns.is_some() { "android:" } else { "" };
                out += &format!("{:w$}A: {ns}{}={:?}\n", "", a.name, a.value, w = d * 2 + 2);
            }
        });
        out
    }
}

pub fn parse(b: &[u8]) -> Result<Element> {
    if u16_at(b, 0)? != RES_XML {
        return Err("not an Android binary XML file".into());
    }
    let end = (u32_at(b, 4)? as usize).min(b.len());
    let mut strings = Vec::new();
    let mut resmap = Vec::new();
    let mut stack = vec![Element::default()]; // synthetic document root
    let mut off = u16_at(b, 2)? as usize;

    while off + 8 <= end {
        let ty = u16_at(b, off)?;
        let header = u16_at(b, off + 2)? as usize;
        let size = u32_at(b, off + 4)? as usize;
        if size < 8 {
            return Err(format!("bad chunk size {size} at 0x{off:x}"));
        }
        let chunk = slice(b, off, size)?;
        match ty {
            RES_STRING_POOL => strings = string_pool(chunk)?,
            RES_XML_RESOURCE_MAP => {
                resmap = (header..size).step_by(4).map(|o| u32_at(chunk, o)).collect::<Result<_>>()?;
            }
            RES_XML_START_ELEMENT => stack.push(start_element(chunk, header, &strings, &resmap)?),
            RES_XML_END_ELEMENT => {
                if stack.len() < 2 {
                    return Err(format!("unbalanced end tag at 0x{off:x}"));
                }
                let el = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(el);
            }
            _ => {} // namespace start/end, CDATA: not needed
        }
        off += size;
    }
    if stack.len() != 1 {
        return Err("unclosed element at end of file".into());
    }
    stack.pop().unwrap().children.into_iter().next().ok_or_else(|| "no root element".into())
}

fn start_element(c: &[u8], ext: usize, strings: &[String], resmap: &[u32]) -> Result<Element> {
    let s = |i: u32| strings.get(i as usize).cloned().unwrap_or_default();
    let attr_start = u16_at(c, ext + 8)? as usize;
    let attr_size = u16_at(c, ext + 10)? as usize;
    let count = u16_at(c, ext + 12)? as usize;
    let mut el = Element { name: s(u32_at(c, ext + 4)?), ..Default::default() };
    for i in 0..count {
        let a = ext + attr_start + i * attr_size;
        let ns = u32_at(c, a)?;
        let name = u32_at(c, a + 4)?;
        let raw = u32_at(c, a + 8)?;
        let ty = u8_at(c, a + 15)?;
        let data = u32_at(c, a + 16)?;
        let value = if raw != NO_INDEX { Value::Str(s(raw)) } else { res_value(ty, data, strings) };
        el.attrs.push(Attr {
            ns: (ns != NO_INDEX).then(|| s(ns)),
            name: s(name),
            res_id: resmap.get(name as usize).copied(),
            value,
        });
    }
    Ok(el)
}

/// ResStringPool chunk → strings. Shared with resources.arsc later.
pub fn string_pool(c: &[u8]) -> Result<Vec<String>> {
    let header = u16_at(c, 2)? as usize;
    let count = u32_at(c, 8)? as usize;
    let utf8 = u32_at(c, 16)? & UTF8_FLAG != 0;
    let start = u32_at(c, 20)? as usize;
    (0..count)
        .map(|i| {
            let p = start + u32_at(c, header + i * 4)? as usize;
            if utf8 {
                let (_, p) = len8(c, p)?; // UTF-16 length, unused
                let (n, p) = len8(c, p)?;
                Ok(String::from_utf8_lossy(slice(c, p, n)?).into_owned())
            } else {
                let (n, p) = len16(c, p)?;
                let units: Vec<u16> = slice(c, p, n * 2)?.chunks_exact(2).map(|u| u16::from_le_bytes([u[0], u[1]])).collect();
                Ok(String::from_utf16_lossy(&units))
            }
        })
        .collect()
}

fn len8(c: &[u8], p: usize) -> Result<(usize, usize)> {
    let a = u8_at(c, p)? as usize;
    if a & 0x80 == 0 { Ok((a, p + 1)) } else { Ok(((a & 0x7f) << 8 | u8_at(c, p + 1)? as usize, p + 2)) }
}

fn len16(c: &[u8], p: usize) -> Result<(usize, usize)> {
    let a = u16_at(c, p)? as usize;
    if a & 0x8000 == 0 { Ok((a, p + 2)) } else { Ok(((a & 0x7fff) << 16 | u16_at(c, p + 2)? as usize, p + 4)) }
}

/// Fully-qualified class name of the MAIN/LAUNCHER activity.
pub fn launcher_activity(manifest: &Element) -> Option<String> {
    let package = manifest.str_attr("package")?;
    let app = manifest.children_named("application").next()?;
    let has = |f: &Element, tag, want| f.children_named(tag).any(|e| e.str_attr("name") == Some(want));
    let act = app
        .children
        .iter()
        .filter(|a| a.name == "activity" || a.name == "activity-alias")
        .find(|a| {
            a.children_named("intent-filter").any(|f| {
                has(f, "action", "android.intent.action.MAIN") && has(f, "category", "android.intent.category.LAUNCHER")
            })
        })?;
    // an alias points at the real class via targetActivity
    let name = act.str_attr("targetActivity").or(act.str_attr("name"))?;
    Some(match name {
        n if n.starts_with('.') => format!("{package}{n}"),
        n if !n.contains('.') => format!("{package}.{n}"),
        n => n.to_string(),
    })
}
