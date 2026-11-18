// Invalid on purpose: returns a u32 from a function declared to return an f32. xtask's
// WGSL gate requires this file to stay refused.
fn f() -> f32 {
    return 1u;
}
