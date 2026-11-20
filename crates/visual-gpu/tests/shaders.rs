//! Validates the libraries' WGSL shaders with naga, without a GPU: the shaders a build of
//! the visual library may read as code (xtask's loaded-files gate reads this file for the
//! list).

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn validates_the_library_s_shaders() {
        // naga, the WGSL front end wgpu compiles with, parses and validates each one without a
        // GPU, with naga's default capabilities (multisampled shading and cube array textures:
        // a shader needing any other is refused): a shader error is its message, with the
        // line, before any pipeline is created. Every shader xtask's loaded-files gate accepts
        // (`VALIDATED_SHADERS`) is one of these, which xtask checks in this file.
        for (name, source) in [
            (
                "crates/visual-boxes/src/square.wgsl",
                visual_boxes::SQUARE_WGSL,
            ),
            (
                "crates/visual-core/src/display.wgsl",
                visual_core::DISPLAY_WGSL,
            ),
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .map_err(|error| error.emit_to_string_with_path(source, name));
            let validated = module.and_then(|parsed| {
                wgpu::naga::valid::Validator::new(
                    wgpu::naga::valid::ValidationFlags::all(),
                    wgpu::naga::valid::Capabilities::default(),
                )
                .validate(&parsed)
                .map(drop)
                .map_err(|error| error.emit_to_string_with_path(source, name))
            });
            assert_eq!(validated, Ok(()), "{name}");
        }
    }
}
