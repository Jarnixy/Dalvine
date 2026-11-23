//! Dalvik VM: heap, class lookup, method dispatch. The opcode loop is in interp.rs.

mod interp;

use crate::{bytes::Result, dex::Dex};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// A host-implemented method (framework/JDK). Gets raw argument slots, returns a raw value.
pub type Native = fn(&mut Vm, &[u32]) -> Result<u64>;

/// shortcut: recursive interpreter, so each Java call costs Rust stack (~19 KB debug, ~1.2 KB release).
/// Host entry calls therefore run on a thread with a big reserved stack; replace with an explicit
/// frame stack if thread spawns per entry call ever show up in profiles.
const MAX_DEPTH: usize = 2000;
const STACK_BYTES: usize = 256 << 20; // reserved address space; only touched pages use memory

pub enum Object {
    Instance { class: String, fields: HashMap<String, u64> },
    /// every element type stored widened to u64
    Array(Vec<u64>),
    Str(String),
}

#[derive(Clone, Copy)]
struct Loc {
    dex: usize,
    class: usize,
    virt: bool,
    i: usize,
}

enum Target {
    Code(Loc),
    Native(Native),
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Invoke {
    Virtual,
    Super,
    Direct,
    Static,
    Interface,
}

pub struct Vm {
    dexes: Arc<[Dex]>,
    /// descriptor → (dex index, class_def index); first dex wins, as on Android
    classes: HashMap<String, (usize, usize)>,
    /// refs are 1-based indices; 0 is null.
    /// shortcut: no GC, everything lives until the Vm is dropped; add mark-and-sweep before long-running apps.
    heap: Vec<Object>,
    statics: HashMap<String, u64>,
    initialized: HashSet<String>,
    natives: HashMap<&'static str, Native>,
    depth: usize,
}

impl Vm {
    pub fn new(dexes: Vec<Dex>) -> Self {
        let mut classes = HashMap::new();
        for (d, dex) in dexes.iter().enumerate() {
            for (c, cd) in dex.classes.iter().enumerate() {
                classes.entry(dex.type_name(cd.class).to_string()).or_insert((d, c));
            }
        }
        Vm {
            dexes: dexes.into(),
            classes,
            heap: vec![],
            statics: HashMap::new(),
            initialized: HashSet::new(),
            natives: crate::runtime::natives(),
            depth: 0,
        }
    }

    /// Entry point from the host: `vm.invoke_static("LCalc;", "add(II)I", &[2, 3])`.
    pub fn invoke_static(&mut self, class: &str, name_proto: &str, args: &[u32]) -> Result<u64> {
        if self.depth > 0 {
            return self.invoke_static_here(class, name_proto, args); // re-entry from a native: already on the big stack
        }
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK_BYTES)
                .spawn_scoped(s, || self.invoke_static_here(class, name_proto, args))
                .map_err(|e| format!("cannot start interpreter thread: {e}"))?
                .join()
                .map_err(|_| "interpreter thread panicked".to_string())?
        })
    }

    fn invoke_static_here(&mut self, class: &str, name_proto: &str, args: &[u32]) -> Result<u64> {
        self.ensure_init(class)?;
        match self.resolve(class, name_proto) {
            Some(Target::Code(loc)) => self.call(loc, args),
            Some(Target::Native(f)) => f(self, args),
            None => Err(format!("MISSING: {class}->{name_proto}")),
        }
    }

    pub fn alloc(&mut self, o: Object) -> u32 {
        self.heap.push(o);
        self.heap.len() as u32
    }

    pub fn obj(&self, r: u32) -> Result<&Object> {
        if r == 0 {
            return Err("NullPointerException".into());
        }
        self.heap.get(r as usize - 1).ok_or_else(|| format!("bad reference {r}"))
    }

    pub fn obj_mut(&mut self, r: u32) -> Result<&mut Object> {
        if r == 0 {
            return Err("NullPointerException".into());
        }
        self.heap.get_mut(r as usize - 1).ok_or_else(|| format!("bad reference {r}"))
    }

    pub fn string(&self, r: u32) -> Result<&str> {
        match self.obj(r)? {
            Object::Str(s) => Ok(s),
            _ => Err(format!("reference {r} is not a String")),
        }
    }

    fn class_of(&self, r: u32) -> Result<String> {
        Ok(match self.obj(r)? {
            Object::Instance { class, .. } => class.clone(),
            Object::Str(_) => "Ljava/lang/String;".into(),
            Object::Array(_) => "Ljava/lang/Object;".into(),
        })
    }

    fn superclass(&self, class: &str) -> Option<String> {
        let &(d, c) = self.classes.get(class)?;
        let dex = &self.dexes[d];
        dex.classes[c].superclass.map(|s| dex.type_name(s).to_string())
    }

    /// shortcut: linear scan per call; cache (class, name_proto) → Target when profiling says so.
    fn declared(&self, class: &str, name_proto: &str) -> Option<Loc> {
        let &(dex, class) = self.classes.get(class)?;
        let d = &self.dexes[dex];
        let cd = &d.classes[class];
        [(false, &cd.direct_methods), (true, &cd.virtual_methods)].into_iter().find_map(|(virt, list)| {
            let i = list.iter().position(|m| d.name_proto(m.method) == name_proto)?;
            Some(Loc { dex, class, virt, i })
        })
    }

    /// Walks `class` and its superclasses: app code first, then host natives for framework classes.
    fn resolve(&self, class: &str, name_proto: &str) -> Option<Target> {
        let mut cls = class.to_string();
        loop {
            if let Some(loc) = self.declared(&cls, name_proto) {
                return Some(Target::Code(loc));
            }
            if let Some(&f) = self.natives.get(format!("{cls}->{name_proto}").as_str()) {
                return Some(Target::Native(f));
            }
            cls = self.superclass(&cls)?;
        }
    }

    /// Runs <clinit> once, superclass first. Classes outside the dex are the host's job.
    fn ensure_init(&mut self, class: &str) -> Result<()> {
        if !self.classes.contains_key(class) || !self.initialized.insert(class.to_string()) {
            return Ok(());
        }
        if let Some(sup) = self.superclass(class) {
            self.ensure_init(&sup)?;
        }
        // shortcut: class_def static_values (constant initializers d8 moves out of <clinit>) are not applied yet
        match self.declared(class, "<clinit>()V") {
            Some(loc) => self.call(loc, &[]).map(drop),
            None => Ok(()),
        }
    }

    /// An invoke-* instruction from method code in dex `di`, running inside class `caller`.
    pub(crate) fn invoke(&mut self, kind: Invoke, di: usize, method: u32, args: &[u32], caller: &str) -> Result<u64> {
        let dex = &self.dexes[di];
        let m = dex.methods.get(method as usize).ok_or_else(|| format!("bad method index {method}"))?;
        let class = dex.type_name(m.class as u32).to_string();
        let np = dex.name_proto(method);
        let start = match kind {
            Invoke::Static | Invoke::Direct => class.clone(),
            Invoke::Virtual | Invoke::Interface => self.class_of(*args.first().ok_or("invoke without receiver")?)?,
            Invoke::Super => self.superclass(caller).ok_or_else(|| format!("{caller} has no superclass"))?,
        };
        if kind == Invoke::Static {
            self.ensure_init(&class)?;
        }
        // a framework receiver (e.g. a String) has no dex chain, so also try the declared class
        match self.resolve(&start, &np).or_else(|| self.resolve(&class, &np)) {
            Some(Target::Code(loc)) => self.call(loc, args),
            Some(Target::Native(f)) => f(self, args),
            None => Err(format!("MISSING: {class}->{np}")),
        }
    }

    fn call(&mut self, loc: Loc, args: &[u32]) -> Result<u64> {
        let dexes = Arc::clone(&self.dexes);
        let dex = &dexes[loc.dex];
        let cd = &dex.classes[loc.class];
        let m = if loc.virt { &cd.virtual_methods[loc.i] } else { &cd.direct_methods[loc.i] };
        let sig = || dex.method_sig(m.method);
        let code = m.code.as_ref().ok_or_else(|| format!("{} has no code (abstract/native)", sig()))?;
        if self.depth >= MAX_DEPTH {
            return Err(format!("StackOverflowError in {}", sig()));
        }
        self.depth += 1;
        let r = interp::run(self, loc.dex, code, dex.type_name(cd.class), args).map_err(|e| format!("{e}\n  at {}", sig()));
        self.depth -= 1;
        r
    }
}
