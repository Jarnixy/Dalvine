//! Runs tests/fixtures/calc/Calc.java (compiled to calc.dex) in our VM. Expected values are what real Java returns.

use apkloader::{dex::Dex, vm::Vm};

fn vm() -> Vm {
    Vm::new(vec![Dex::parse(&std::fs::read("tests/fixtures/calc.dex").unwrap()).unwrap()])
}

fn int(vm: &mut Vm, np: &str, args: &[u32]) -> i32 {
    vm.invoke_static("LCalc;", np, args).unwrap_or_else(|e| panic!("{np}: {e}")) as i32
}

#[test]
fn milestone_4_add() {
    assert_eq!(int(&mut vm(), "add(II)I", &[2, 3]), 5);
}

#[test]
fn arithmetic_and_control_flow() {
    let vm = &mut vm();
    assert_eq!(int(vm, "add(II)I", &[-7i32 as u32, 3]), -4);
    assert_eq!(int(vm, "add(II)I", &[i32::MAX as u32, 1]), i32::MIN); // wraps like Java
    assert_eq!(int(vm, "fib(I)I", &[15]), 610);
    assert_eq!(int(vm, "sumTo(I)I", &[100]), 5050);
    assert_eq!(int(vm, "div(II)I", &[-7i32 as u32, 2]), -3);
    for x in [0, 1, 5, -5, 12345, i32::MIN] {
        let want = ((x << 3) ^ ((x as u32) >> 1) as i32) & 0xff | (x.wrapping_neg() >> 31);
        assert_eq!(int(vm, "bits(I)I", &[x as u32]), want, "bits({x})");
    }
}

#[test]
fn objects_fields_and_dispatch() {
    let vm = &mut vm();
    assert_eq!(int(vm, "objects()I", &[]), 42); // new-instance, constructor, iput/iget, invoke-virtual
    assert_eq!(int(vm, "bump()I", &[]), 5); // sget/sput
    assert_eq!(int(vm, "bump()I", &[]), 10); // static state persists
    assert_eq!(int(vm, "seeded()I", &[]), 42); // <clinit> ran
    assert_eq!(int(vm, "virt()I", &[]), 31); // override + invoke-super
    assert_eq!(int(vm, "arr()I", &[]), 7 + 3 + (200u8 as i8 as i32)); // arrays, byte truncation
}

#[test]
fn wide_and_strings() {
    let vm = &mut vm();
    let v = -0x1234_5678_9abc_def0i64 as u64;
    assert_eq!(vm.invoke_static("LCalc;", "wide(J)J", &[v as u32, (v >> 32) as u32]).unwrap(), v);
    let s = vm.invoke_static("LCalc;", "hello()Ljava/lang/String;", &[]).unwrap();
    assert_eq!(vm.string(s as u32).unwrap(), "hi");
}

#[test]
fn failures_are_errors_with_a_trace() {
    let vm = &mut vm();
    let e = vm.invoke_static("LCalc;", "div(II)I", &[1, 0]).unwrap_err();
    assert!(e.starts_with("ArithmeticException") && e.contains("at LCalc;->div(II)I"), "{e}");
    let e = vm.invoke_static("LCalc;", "missing()I", &[]).unwrap_err();
    assert!(e.starts_with("MISSING: Ljava/lang/Integer;->parseInt(Ljava/lang/String;)I"), "{e}");
    let e = vm.invoke_static("LCalc;", "deep(I)I", &[100_000]).unwrap_err();
    assert!(e.starts_with("StackOverflowError"), "{e}");
    assert_eq!(int(vm, "deep(I)I", &[1500]), 1500); // deep but legal recursion works, and the VM is still usable
    assert!(vm.invoke_static("LCalc;", "add(II)I", &[1]).is_err()); // wrong arg count
}
