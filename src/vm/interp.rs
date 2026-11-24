//! The bytecode loop. Registers are raw 32-bit slots, as in Dalvik; wide values use a pair (low, high).
//! Opcode reference: https://source.android.com/docs/core/runtime/dalvik-bytecode

use super::{Invoke, Object, Vm};
use crate::{bytes::Result, dex::Code};
use std::{collections::HashMap, sync::Arc};

/// Largest array new-array may create, so hostile bytecode can't exhaust host memory.
const MAX_ARRAY: usize = 1 << 26;
const INVOKE_KINDS: [Invoke; 5] = [Invoke::Virtual, Invoke::Super, Invoke::Direct, Invoke::Static, Invoke::Interface];

pub(super) fn run(vm: &mut Vm, di: usize, code: &Code, class: &str, args: &[u32]) -> Result<u64> {
    let dexes = Arc::clone(&vm.dexes);
    let dex = &dexes[di];
    let ins = &code.insns[..];
    let n = code.registers as usize;
    if args.len() != code.ins as usize || args.len() > n {
        return Err(format!("got {} argument slots, method takes {} of {n} registers", args.len(), code.ins));
    }
    let mut r = vec![0u32; n];
    r[n - args.len()..].copy_from_slice(args); // arguments live in the last registers
    let mut result = 0u64; // for move-result*
    let mut pc = 0usize;

    loop {
        let u = at(ins, pc)?;
        let op = u & 0xff;
        let (a4, b4, aa) = (((u >> 8) & 0xf) as usize, (u >> 12) as usize, (u >> 8) as usize);
        pc = match op {
            0x00 => pc + 1, // nop
            // move, move-object: vA, vB / from16 / 16
            0x01 | 0x07 => { let v = get(&r, b4)?; put(&mut r, a4, v)?; pc + 1 }
            0x02 | 0x08 => { let v = get(&r, at(ins, pc + 1)? as usize)?; put(&mut r, aa, v)?; pc + 2 }
            0x03 | 0x09 => { let v = get(&r, at(ins, pc + 2)? as usize)?; put(&mut r, at(ins, pc + 1)? as usize, v)?; pc + 3 }
            0x04 => { let v = get2(&r, b4)?; put2(&mut r, a4, v)?; pc + 1 }
            0x05 => { let v = get2(&r, at(ins, pc + 1)? as usize)?; put2(&mut r, aa, v)?; pc + 2 }
            0x06 => { let v = get2(&r, at(ins, pc + 2)? as usize)?; put2(&mut r, at(ins, pc + 1)? as usize, v)?; pc + 3 }
            // move-result, move-result-wide, move-result-object
            0x0a | 0x0c => { put(&mut r, aa, result as u32)?; pc + 1 }
            0x0b => { put2(&mut r, aa, result)?; pc + 1 }
            // return-void, return, return-wide, return-object
            0x0e => return Ok(0),
            0x0f | 0x11 => return Ok(get(&r, aa)? as u64),
            0x10 => return get2(&r, aa),
            // const/4, const/16, const, const/high16
            0x12 => { put(&mut r, a4, ((u as i16) >> 12) as u32)?; pc + 1 }
            0x13 => { put(&mut r, aa, at(ins, pc + 1)? as i16 as u32)?; pc + 2 }
            0x14 => { put(&mut r, aa, imm32(ins, pc)?)?; pc + 3 }
            0x15 => { put(&mut r, aa, (at(ins, pc + 1)? as u32) << 16)?; pc + 2 }
            // const-wide/16, /32, const-wide, /high16
            0x16 => { put2(&mut r, aa, at(ins, pc + 1)? as i16 as i64 as u64)?; pc + 2 }
            0x17 => { put2(&mut r, aa, imm32(ins, pc)? as i32 as i64 as u64)?; pc + 3 }
            0x18 => {
                let v = (0..4).try_fold(0u64, |v, i| Ok::<_, String>(v | (at(ins, pc + 1 + i)? as u64) << (16 * i)))?;
                put2(&mut r, aa, v)?;
                pc + 5
            }
            0x19 => { put2(&mut r, aa, (at(ins, pc + 1)? as u64) << 48)?; pc + 2 }
            // const-string, const-string/jumbo
            0x1a | 0x1b => {
                let (idx, len) = if op == 0x1a { (at(ins, pc + 1)? as u32, 2) } else { (imm32(ins, pc)?, 3) };
                let s = vm.alloc(Object::Str(dex.string(idx).to_string()));
                put(&mut r, aa, s)?;
                pc + len
            }
            // check-cast vAA, type: null always passes
            0x1f => {
                let (obj, ty) = (get(&r, aa)?, dex.type_name(at(ins, pc + 1)? as u32));
                if obj != 0 && !vm.instance_of(obj, ty)? {
                    return Err(format!("ClassCastException: {} cannot be cast to {ty}", vm.class_of(obj)?));
                }
                pc + 2
            }
            // instance-of vA, vB, type
            0x20 => {
                let obj = get(&r, b4)?;
                let yes = obj != 0 && vm.instance_of(obj, dex.type_name(at(ins, pc + 1)? as u32))?;
                put(&mut r, a4, yes as u32)?;
                pc + 2
            }
            // array-length
            0x21 => {
                let len = match vm.obj(get(&r, b4)?)? { Object::Array(v) => v.len(), _ => return Err("array-length on non-array".into()) };
                put(&mut r, a4, len as u32)?;
                pc + 1
            }
            // new-instance
            0x22 => {
                let cls = dex.type_name(at(ins, pc + 1)? as u32).to_string();
                vm.ensure_init(&cls)?;
                let o = vm.alloc(Object::Instance { class: cls, fields: HashMap::new() });
                put(&mut r, aa, o)?;
                pc + 2
            }
            // new-array
            0x23 => {
                let len = get(&r, b4)? as i32;
                let len = usize::try_from(len).map_err(|_| format!("NegativeArraySizeException: {len}"))?;
                if len > MAX_ARRAY {
                    return Err(format!("OutOfMemoryError: array of {len}"));
                }
                let o = vm.alloc(Object::Array(vec![0; len]));
                put(&mut r, a4, o)?;
                pc + 2
            }
            0x27 => return Err("uncaught exception (throw is not supported yet)".into()),
            // goto, goto/16, goto/32
            0x28 => jump(pc, (u >> 8) as u8 as i8 as i32)?,
            0x29 => jump(pc, at(ins, pc + 1)? as i16 as i32)?,
            0x2a => jump(pc, imm32(ins, pc)? as i32)?,
            // if-eq .. if-le vA, vB
            0x32..=0x37 => {
                if cmp(op - 0x32, get(&r, a4)? as i32, get(&r, b4)? as i32) { jump(pc, at(ins, pc + 1)? as i16 as i32)? } else { pc + 2 }
            }
            // if-eqz .. if-lez vAA
            0x38..=0x3d => {
                if cmp(op - 0x38, get(&r, aa)? as i32, 0) { jump(pc, at(ins, pc + 1)? as i16 as i32)? } else { pc + 2 }
            }
            // aget*, aput*: vAA, vBB (array), vCC (index)
            0x44..=0x51 => {
                let w = at(ins, pc + 1)?;
                let (arr, idx) = (get(&r, (w & 0xff) as usize)?, get(&r, (w >> 8) as usize)? as i32);
                let wide = op == 0x45 || op == 0x4c;
                let value = match op {
                    0x44..=0x4a => None,
                    _ if wide => Some(get2(&r, aa)?),
                    _ => Some(narrow(op - 0x4b, get(&r, aa)?) as u64),
                };
                let Object::Array(v) = vm.obj_mut(arr)? else { return Err("aget/aput on non-array".into()) };
                let len = v.len();
                let slot = usize::try_from(idx).ok().and_then(|i| v.get_mut(i))
                    .ok_or_else(|| format!("ArrayIndexOutOfBoundsException: index {idx}, length {len}"))?;
                match value {
                    Some(x) => *slot = x,
                    None if wide => { let x = *slot; put2(&mut r, aa, x)? }
                    None => { let x = *slot as u32; put(&mut r, aa, x)? }
                }
                pc + 2
            }
            // iget*, iput*: vA, vB (object), field
            0x52..=0x5f => {
                let key = dex.field_key(at(ins, pc + 1)? as u32);
                let wide = op == 0x53 || op == 0x5a;
                let value = match op {
                    0x52..=0x58 => None,
                    _ if wide => Some(get2(&r, a4)?),
                    _ => Some(narrow(op - 0x59, get(&r, a4)?) as u64),
                };
                let Object::Instance { fields, .. } = vm.obj_mut(get(&r, b4)?)? else { return Err("iget/iput on non-instance".into()) };
                match value {
                    Some(x) => { fields.insert(key, x); }
                    None => { let x = fields.get(&key).copied().unwrap_or(0); if wide { put2(&mut r, a4, x)? } else { put(&mut r, a4, x as u32)? } }
                }
                pc + 2
            }
            // sget*, sput*: vAA, field
            0x60..=0x6d => {
                let fi = at(ins, pc + 1)? as u32;
                let field = dex.fields.get(fi as usize).ok_or_else(|| format!("bad field index {fi}"))?;
                vm.ensure_init(dex.type_name(field.class as u32))?;
                // shortcut: fields are keyed by the class named in the instruction, not resolved up the hierarchy
                let key = dex.field_key(fi);
                let wide = op == 0x61 || op == 0x68;
                match op {
                    0x60..=0x66 => { let x = vm.statics.get(&key).copied().unwrap_or(0); if wide { put2(&mut r, aa, x)? } else { put(&mut r, aa, x as u32)? } }
                    _ => { let x = if wide { get2(&r, aa)? } else { narrow(op - 0x67, get(&r, aa)?) as u64 }; vm.statics.insert(key, x); }
                }
                pc + 2
            }
            // invoke-kind {vC, vD, vE, vF, vG}, method
            0x6e..=0x72 => {
                let (count, w) = (b4, at(ins, pc + 2)? as usize);
                if count > 5 {
                    return Err(format!("invoke with {count} registers"));
                }
                let regs = [w & 0xf, (w >> 4) & 0xf, (w >> 8) & 0xf, w >> 12, a4];
                let args = regs[..count].iter().map(|&i| get(&r, i)).collect::<Result<Vec<_>>>()?;
                result = vm.invoke(INVOKE_KINDS[(op - 0x6e) as usize], di, at(ins, pc + 1)? as u32, &args, class)?;
                pc + 3
            }
            // invoke-kind/range {vCCCC .. vNNNN}, method
            0x74..=0x78 => {
                let first = at(ins, pc + 2)? as usize;
                let args = (first..first + aa).map(|i| get(&r, i)).collect::<Result<Vec<_>>>()?;
                result = vm.invoke(INVOKE_KINDS[(op - 0x74) as usize], di, at(ins, pc + 1)? as u32, &args, class)?;
                pc + 3
            }
            // unary ops vA, vB
            0x7b | 0x7c | 0x81 | 0x84 | 0x8d..=0x8f => {
                if op == 0x81 {
                    let x = get(&r, b4)? as i32 as i64 as u64; // int-to-long
                    put2(&mut r, a4, x)?;
                } else {
                    let x = if op == 0x84 { get2(&r, b4)? as u32 } else { get(&r, b4)? };
                    let v = match op {
                        0x7b => (x as i32).wrapping_neg() as u32,
                        0x7c => !x,
                        0x84 => x, // long-to-int keeps the low half
                        0x8d => x as i8 as u32,
                        0x8e => x as u16 as u32,
                        _ => x as i16 as u32,
                    };
                    put(&mut r, a4, v)?;
                }
                pc + 1
            }
            // binop-int vAA, vBB, vCC
            0x90..=0x9a => {
                let w = at(ins, pc + 1)?;
                let v = binop(op - 0x90, get(&r, (w & 0xff) as usize)?, get(&r, (w >> 8) as usize)?)?;
                put(&mut r, aa, v)?;
                pc + 2
            }
            // binop-int/2addr vA, vB
            0xb0..=0xba => { let v = binop(op - 0xb0, get(&r, a4)?, get(&r, b4)?)?; put(&mut r, a4, v)?; pc + 1 }
            // binop-int/lit16 vA, vB, #CCCC (slot 1 is rsub)
            0xd0..=0xd7 => {
                let v = lit_op(op - 0xd0, get(&r, b4)?, at(ins, pc + 1)? as i16 as u32)?;
                put(&mut r, a4, v)?;
                pc + 2
            }
            // binop-int/lit8 vAA, vBB, #CC
            0xd8..=0xe2 => {
                let w = at(ins, pc + 1)?;
                let v = lit_op(op - 0xd8, get(&r, (w & 0xff) as usize)?, ((w as i16) >> 8) as u32)?;
                put(&mut r, aa, v)?;
                pc + 2
            }
            _ => return Err(format!("unsupported opcode 0x{op:02x} at pc {pc}")),
        };
    }
}

fn at(ins: &[u16], i: usize) -> Result<u16> {
    ins.get(i).copied().ok_or_else(|| format!("instruction at {i} past end of method"))
}

fn imm32(ins: &[u16], pc: usize) -> Result<u32> {
    Ok(at(ins, pc + 1)? as u32 | (at(ins, pc + 2)? as u32) << 16)
}

fn get(r: &[u32], i: usize) -> Result<u32> {
    r.get(i).copied().ok_or_else(|| format!("register v{i} out of range"))
}

fn put(r: &mut [u32], i: usize, v: u32) -> Result<()> {
    *r.get_mut(i).ok_or_else(|| format!("register v{i} out of range"))? = v;
    Ok(())
}

fn get2(r: &[u32], i: usize) -> Result<u64> {
    Ok(get(r, i)? as u64 | (get(r, i + 1)? as u64) << 32)
}

fn put2(r: &mut [u32], i: usize, v: u64) -> Result<()> {
    put(r, i + 1, (v >> 32) as u32)?;
    put(r, i, v as u32)
}

fn jump(pc: usize, off: i32) -> Result<usize> {
    pc.checked_add_signed(off as isize).ok_or_else(|| format!("branch to before start of method from pc {pc}"))
}

fn cmp(k: u16, x: i32, y: i32) -> bool {
    match k {
        0 => x == y,
        1 => x != y,
        2 => x < y,
        3 => x >= y,
        4 => x > y,
        _ => x <= y,
    }
}

/// Store-side truncation for the typed put variants: 0 plain, 1 wide, 2 object, 3 boolean, 4 byte, 5 char, 6 short.
fn narrow(k: u16, x: u32) -> u32 {
    match k {
        3 => x & 1,
        4 => x as i8 as u32,
        5 => x as u16 as u32,
        6 => x as i16 as u32,
        _ => x,
    }
}

/// add sub mul div rem and or xor shl shr ushr, with Java's int overflow semantics.
fn binop(k: u16, x: u32, y: u32) -> Result<u32> {
    let (a, b) = (x as i32, y as i32);
    if (k == 3 || k == 4) && b == 0 {
        return Err("ArithmeticException: divide by zero".into());
    }
    Ok(match k {
        0 => a.wrapping_add(b),
        1 => a.wrapping_sub(b),
        2 => a.wrapping_mul(b),
        3 => a.wrapping_div(b),
        4 => a.wrapping_rem(b),
        5 => a & b,
        6 => a | b,
        7 => a ^ b,
        8 => a.wrapping_shl(y & 31),
        9 => a.wrapping_shr(y & 31),
        _ => (x >> (y & 31)) as i32,
    } as u32)
}

/// The /lit opcodes use binop's numbering, except slot 1 is reverse-subtract.
fn lit_op(k: u16, x: u32, lit: u32) -> Result<u32> {
    if k == 1 { Ok((lit as i32).wrapping_sub(x as i32) as u32) } else { binop(k, x, lit) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_int_semantics() {
        assert_eq!(binop(3, i32::MIN as u32, -1i32 as u32).unwrap(), i32::MIN as u32); // no panic on MIN / -1
        assert_eq!(binop(4, i32::MIN as u32, -1i32 as u32).unwrap(), 0);
        assert_eq!(binop(9, -8i32 as u32, 1).unwrap(), -4i32 as u32); // shr is arithmetic
        assert_eq!(binop(10, -8i32 as u32, 1).unwrap(), 0x7fff_fffc); // ushr is logical
        assert_eq!(binop(8, 1, 33).unwrap(), 2); // shift distance masked to 5 bits
        assert!(binop(3, 1, 0).is_err());
        assert_eq!(lit_op(1, 3, 10).unwrap(), 7); // rsub
    }
}
