//! Host implementations of JDK/Android methods, keyed by `Lclass;->name(params)ret`.
//! Anything not here and not in the app's dex fails with `MISSING: <signature>`: that's the to-do list.

use crate::vm::Native;
use std::collections::HashMap;

pub fn natives() -> HashMap<&'static str, Native> {
    HashMap::from([("Ljava/lang/Object;-><init>()V", (|_, _| Ok(0)) as Native)])
}
