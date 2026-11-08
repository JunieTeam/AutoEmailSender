// The app links the VC runtime in, as tauri-build does by default; this command ships beside it
// and must start too on a Windows without the Visual C++ Redistributable.
fn main() {
    static_vcruntime::metabuild();
}
