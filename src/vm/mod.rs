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

    /// Host entry: `vm.invoke_static("LCalc;", "add(II)I", &[2, 3])`.
    pub fn invoke_static(&mut self, class: &str, name_proto: &str, args: &[u32]) -> Result<u64> {
        self.entry(|vm| {
            vm.ensure_init(class)?;
            vm.dispatch(class, class, name_proto, args)
        })
    }

    /// Host entry: calls `name_proto` on `args[0]`, dispatching on its runtime class.
    pub fn invoke_virtual(&mut self, name_proto: &str, args: &[u32]) -> Result<u64> {
        self.entry(|vm| {
            let class = vm.class_of(*args.first().ok_or("invoke_virtual without receiver")?)?;
            vm.dispatch(&class, &class, name_proto, args)
        })
    }

    /// Host entry: `new class()`.
    pub fn new_object(&mut self, class: &str) -> Result<u32> {
        self.entry(|vm| {
            vm.ensure_init(class)?;
            let r = vm.alloc(Object::Instance { class: class.into(), fields: HashMap::new() });
            vm.dispatch(class, class, "<init>()V", &[r])?;
            Ok(r)
        })
    }

    /// Runs host calls on a big stack; nested calls (natives calling back into Java) are already on it.
    fn entry<T: Send>(&mut self, f: impl FnOnce(&mut Vm) -> Result<T> + Send) -> Result<T> {
        if self.depth > 0 {
            return f(self);
        }
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK_BYTES)
                .spawn_scoped(s, || f(self))
                .map_err(|e| format!("cannot start interpreter thread: {e}"))?
                .join()
                .map_err(|_| "interpreter thread panicked".to_string())?
        })
    }

    /// Looks the method up from `start`, falling back to the `declared` class: a framework receiver
    /// (e.g. a String) has no app-code chain to walk.
    fn dispatch(&mut self, start: &str, declared: &str, name_proto: &str, args: &[u32]) -> Result<u64> {
        match self.resolve(start, name_proto).or_else(|| self.resolve(declared, name_proto)) {
            Some(Target::Code(loc)) => self.call(loc, args),
            Some(Target::Native(f)) => f(self, args),
            None => Err(format!("MISSING: {declared}->{name_proto}")),
        }
    }

    /// Host-side state on an object (framework fields like a TextView's text). Absent = None.
    pub fn field(&self, r: u32, key: &str) -> Result<Option<u64>> {
        match self.obj(r)? {
            Object::Instance { fields, .. } => Ok(fields.get(key).copied()),
            _ => Err(format!("reference {r} has no fields")),
        }
    }

    pub fn set_field(&mut self, r: u32, key: &str, v: u64) -> Result<()> {
        match self.obj_mut(r)? {
            Object::Instance { fields, .. } => {
                fields.insert(key.into(), v);
                Ok(())
            }
            _ => Err(format!("reference {r} has no fields")),
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

    pub fn class_of(&self, r: u32) -> Result<String> {
        Ok(match self.obj(r)? {
            Object::Instance { class, .. } => class.clone(),
            Object::Str(_) => "Ljava/lang/String;".into(),
            Object::Array(_) => "Ljava/lang/Object;".into(),
        })
    }

    /// App classes answer from the dex; framework classes from the runtime's hierarchy table.
    fn superclass(&self, class: &str) -> Option<String> {
        let Some(&(d, c)) = self.classes.get(class) else { return crate::runtime::superclass(class).map(Into::into) };
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
        for _ in 0..64 {
            // bounded: a malformed dex can declare a superclass cycle
            if let Some(loc) = self.declared(&cls, name_proto) {
                return Some(Target::Code(loc));
            }
            if let Some(&f) = self.natives.get(format!("{cls}->{name_proto}").as_str()) {
                return Some(Target::Native(f));
            }
            cls = self.superclass(&cls)?;
        }
        None
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
        self.dispatch(&start, &class, &np, args)
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
