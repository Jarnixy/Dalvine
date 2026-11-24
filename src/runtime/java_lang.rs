//! java.lang : String, StringBuilder, Integer/Long parsing.

use super::{arg, jstr};
use crate::{
    bytes::Result,
    vm::{Native, Object, Vm},
};

/// A StringBuilder's text lives in a String heap object, mutated in place.
const SB: &str = "@sb";

pub fn natives() -> Vec<(&'static str, Native)> {
    vec![
        ("Ljava/lang/StringBuilder;-><init>()V", |vm, a| sb_init(vm, arg(a, 0)?, String::new())),
        ("Ljava/lang/StringBuilder;-><init>(Ljava/lang/String;)V", |vm, a| {
            let s = vm.string(arg(a, 1)?)?.to_string(); // null throws NPE, as in Java
            sb_init(vm, arg(a, 0)?, s)
        }),
        ("Ljava/lang/StringBuilder;->append(Ljava/lang/String;)Ljava/lang/StringBuilder;", |vm, a| {
            let s = jstr(vm, arg(a, 1)?)?; // null appends "null"
            sb_push(vm, arg(a, 0)?, &s)
        }),
        ("Ljava/lang/StringBuilder;->append(I)Ljava/lang/StringBuilder;", |vm, a| {
            sb_push(vm, arg(a, 0)?, &(arg(a, 1)? as i32).to_string())
        }),
        ("Ljava/lang/StringBuilder;->toString()Ljava/lang/String;", |vm, a| {
            let s = sb_text(vm, arg(a, 0)?)?.to_string();
            Ok(vm.alloc(Object::Str(s)) as u64)
        }),
        ("Ljava/lang/String;->length()I", |vm, a| Ok(vm.string(arg(a, 0)?)?.encode_utf16().count() as u64)),
        ("Ljava/lang/Integer;->parseInt(Ljava/lang/String;)I", |vm, a| {
            let s = vm.string(arg(a, 0)?)?;
            s.parse::<i32>().map(|v| v as u32 as u64).map_err(|_| number_format(s))
        }),
        ("Ljava/lang/Long;->parseLong(Ljava/lang/String;)J", |vm, a| {
            let s = vm.string(arg(a, 0)?)?;
            s.parse::<i64>().map(|v| v as u64).map_err(|_| number_format(s))
        }),
    ]
}

fn number_format(s: &str) -> String {
    format!("NumberFormatException: For input string: {s:?}")
}

fn sb_init(vm: &mut Vm, this: u32, s: String) -> Result<u64> {
    let text = vm.alloc(Object::Str(s));
    vm.set_field(this, SB, text as u64)?;
    Ok(0)
}

fn sb_text(vm: &Vm, this: u32) -> Result<&str> {
    let text = vm.field(this, SB)?.ok_or("StringBuilder used before its constructor")?;
    vm.string(text as u32)
}

/// Appends and returns `this`, so chained appends work.
fn sb_push(vm: &mut Vm, this: u32, s: &str) -> Result<u64> {
    let text = vm.field(this, SB)?.ok_or("StringBuilder used before its constructor")?;
    if let Object::Str(buf) = vm.obj_mut(text as u32)? {
        buf.push_str(s);
    }
    Ok(this as u64)
}
