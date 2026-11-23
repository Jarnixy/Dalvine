//! Bounds-checked little-endian reads. Every offset in an APK is untrusted input.

pub type Result<T> = std::result::Result<T, String>;

pub fn slice(b: &[u8], off: usize, len: usize) -> Result<&[u8]> {
    off.checked_add(len)
        .and_then(|end| b.get(off..end))
        .ok_or_else(|| format!("read of {len} bytes at 0x{off:x} past end (size 0x{:x})", b.len()))
}

pub fn u8_at(b: &[u8], off: usize) -> Result<u8> {
    Ok(slice(b, off, 1)?[0])
}

pub fn u16_at(b: &[u8], off: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(b, off, 2)?.try_into().unwrap()))
}

pub fn u32_at(b: &[u8], off: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(b, off, 4)?.try_into().unwrap()))
}

/// Reads a ULEB128 value at `*off` and advances past it.
pub fn uleb128(b: &[u8], off: &mut usize) -> Result<u32> {
    let mut v = 0u32;
    for i in 0..5 {
        let byte = u8_at(b, *off)?;
        *off += 1;
        v |= ((byte & 0x7f) as u32) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(format!("uleb128 longer than 5 bytes before 0x{:x}", *off))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb() {
        for (bytes, want) in [(&[0x00][..], 0), (&[0x7f], 127), (&[0x80, 0x7f], 16256), (&[0xff, 0xff, 0xff, 0xff, 0x0f], u32::MAX)] {
            let mut off = 0;
            assert_eq!(uleb128(bytes, &mut off).unwrap(), want);
            assert_eq!(off, bytes.len());
        }
        assert!(uleb128(&[0x80], &mut 0).is_err());
    }

    #[test]
    fn out_of_bounds_is_error_not_panic() {
        assert!(u32_at(&[1, 2, 3], 0).is_err());
        assert!(slice(&[0; 4], usize::MAX, 2).is_err());
    }
}
