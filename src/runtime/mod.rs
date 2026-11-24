//! Host implementations of JDK/Android methods, keyed by `Lclass;->name(params)ret`.
//! Anything not here and not in the app's dex fails with `MISSING: <signature>`: that's the to-do list.

pub mod android;
pub mod java_lang;

use crate::{
    bytes::Result,
    vm::{Native, Object, Vm},
};
use std::collections::HashMap;

pub fn natives() -> HashMap<&'static str, Native> {
    let mut m: HashMap<&'static str, Native> = HashMap::from([("Ljava/lang/Object;-><init>()V", (|_, _| Ok(0)) as Native)]);
    m.extend(android::natives());
    m.extend(java_lang::natives());
    m
}

/// Parent of a framework class (app classes get theirs from the dex). Unknown classes sit under Object.
pub fn superclass(class: &str) -> Option<&'static str> {
    Some(match class {
        "Ljava/lang/Object;" => return None,
        "Landroid/app/Activity;" => "Landroid/content/Context;",
        "Landroid/widget/Button;" => "Landroid/widget/TextView;",
        "Landroid/widget/TextView;" | "Landroid/view/ViewGroup;" => "Landroid/view/View;",
        "Landroid/widget/LinearLayout;" => "Landroid/view/ViewGroup;",
        "Landroid/widget/LinearLayout$LayoutParams;" => "Landroid/view/ViewGroup$MarginLayoutParams;",
        "Landroid/view/ViewGroup$MarginLayoutParams;" => "Landroid/view/ViewGroup$LayoutParams;",
        _ => "Ljava/lang/Object;",
    })
}

/// Argument slot `i`; malformed bytecode can pass fewer registers than the signature says.
pub fn arg(a: &[u32], i: usize) -> Result<u32> {
    a.get(i).copied().ok_or_else(|| format!("native called with {} args, needs at least {}", a.len(), i + 1))
}

/// Java-style text for any reference: the string itself, "null", or Class@ref.
pub fn jstr(vm: &Vm, r: u32) -> Result<String> {
    if r == 0 {
        return Ok("null".into());
    }
    Ok(match vm.obj(r)? {
        Object::Str(s) => s.clone(),
        _ => format!("{}@{r}", vm.class_of(r)?),
    })
}
