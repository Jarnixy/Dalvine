// Interpreter test program. Rebuild calc.dex with tools/build_calc.sh after editing.
public class Calc {
    static int counter;
    static int seeded = 6 * seed(); // forces a <clinit>
    int value;

    Calc(int v) { value = v; }

    static int seed() { return 7; }
    public static int add(int a, int b) { return a + b; }
    public static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
    public static int sumTo(int n) { int s = 0; for (int i = 1; i <= n; i++) s += i; return s; }
    public static int div(int a, int b) { return a / b; }
    public static int bits(int x) { return ((x << 3) ^ (x >>> 1)) & 0xff | (-x >> 31); }
    public int get() { return value; }
    public static int objects() { Calc c = new Calc(40); return c.get() + 2; }
    public static int bump() { counter += 5; return counter; }
    public static int seeded() { return seeded; }
    public static long wide(long a) { return a; }
    public static String hello() { return "hi"; }
    public static int virt() { Base b = new Sub(); return b.f() * 10 + new Base().f(); }
    public static int arr() { int[] a = new int[3]; a[1] = 7; byte[] b = new byte[1]; b[0] = (byte) 200; return a[1] + a.length + b[0]; }
    public static int deep(int n) { return n == 0 ? 0 : 1 + deep(n - 1); }
    public static long lmath(long a, long b) { return (a * b) % 1000000007L + (a >> 3) - (b >>> 60) + (a ^ b) / 7; }
    public static int fmath(int x) { float f = x / 3f; double d = f * 2.5; return (int) d + (int) -f + (int) (d % 4.0); }
    public static int cmps(double a, double b) { return a < b ? -1 : a > b ? 1 : a == b ? 0 : 9; }
    public static int fcmps(float a, float b) { return a > b ? 1 : 0; }
    public static long conv(int x) { return (long) (float) x + (long) (double) x + (int) 3.9e10 + (long) Float.NaN; }
    public static int missing() { return Integer.toBinaryString(5).length(); }

    static class Base { int f() { return 1; } }
    static class Sub extends Base { int f() { return 2 + super.f(); } }
}
