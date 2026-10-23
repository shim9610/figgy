// The shared surface material of bars, boxes and sectors (SHADER_COMMON.md
// §5.9). Rust twin: `data_render::MaterialGpu` and the `MATERIAL_*`
// constants in `data_render/style.rs`.
const MATERIAL_FLAT: u32 = 0u;
const MATERIAL_MATTE: u32 = 1u;
const MATERIAL_CERAMIC: u32 = 2u;
const MATERIAL_BRUSHED_METAL: u32 = 3u;
const MATERIAL_PAPER: u32 = 4u;
const MATERIAL_WOOD: u32 = 5u;
const MATERIAL_SATIN_METAL: u32 = 6u;
const MATERIAL_TOON: u32 = 7u;
const MATERIAL_ENAMEL: u32 = 8u;
const MATERIAL_HATCH: u32 = 9u;
const MATERIAL_PEARL: u32 = 10u;
struct Material {
    params: vec4<f32>, // kind, roughness, texture strength, texture scale
    finish: vec4<f32>, // gloss, seed, texture angle (radians), emphasis brightness
};
