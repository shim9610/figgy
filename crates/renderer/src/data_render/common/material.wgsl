// The shared surface material of bars, boxes and sectors (SHADER_COMMON.md
// §5.9). Rust twin of the struct and constants: `data_render::MaterialGpu`
// and the `MATERIAL_*` constants in `data_render/style.rs`.
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
fn hash(p:vec2<f32>)->f32 { return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
fn noise(p:vec2<f32>)->f32 { let i=floor(p);let f=fract(p);let u=f*f*(3.0-2.0*f);return mix(mix(hash(i),hash(i+vec2(1.0,0.0)),u.x),mix(hash(i+vec2(0.0,1.0)),hash(i+1.0),u.x),u.y); }
// Continuous solid noise: rough surfaces must vary in z on an extruded wall, not
// repeat a top-plane sample down its entire height. No face UV seams.
fn material_noise(p: vec3<f32>) -> f32 {
    let z=floor(p.z);let f=fract(p.z);let blend=f*f*(3.0-2.0*f);
    return mix(noise(p.xy+z*vec2(37.0,91.0)),noise(p.xy+(z+1.0)*vec2(37.0,91.0)),blend);
}
fn material_band(p: vec3<f32>,frequency: vec3<f32>,pixels: f32) -> f32 {
    // Keep coordinates fixed in object space. Fade frequencies that cannot be
    // resolved at this output size/angle instead of changing the pattern scale.
    let cycles=max(max(frequency.x,frequency.y),frequency.z)/max(pixels,1.0);
    let resolved=1.0-smoothstep(0.3,0.85,cycles);
    return (material_noise(p*frequency)-0.5)*resolved;
}
// The lit color of a surface point. `p` is the object-space point (one unit
// is `pixels` output pixels: the sector radius, or `BAR_UNIT_PX` for a bar),
// `n` the unit normal, `view` the unit direction to the viewer and `light`
// the unit direction to the light. The function turns `p` by the texture
// angle and scales it by the texture scale. `tangent` is the unit brush
// direction of BrushedMetal and SatinMetal on the surface, and `brush` their
// (across, along) brush coordinates: the noise runs at a pixel-limited
// frequency across the brush and at `brush.y` along it.
fn material_surface(m: Material,color: vec3<f32>,p: vec3<f32>,n: vec3<f32>,view: vec3<f32>,light: vec3<f32>,tangent: vec3<f32>,brush: vec2<f32>,pixels: f32) -> vec3<f32> {
    let kind=u32(m.params.x);
    if kind==MATERIAL_FLAT { return min(color+vec3(m.finish.w),vec3(1.0)); }
    let l=light;let half_dir=normalize(l+view);
    let diffuse=max(dot(n,l),0.0);
    let ca=cos(m.finish.z);let sa=sin(m.finish.z);
    let uv=vec2(p.x*ca-p.y*sa,p.x*sa+p.y*ca)*m.params.w;
    let grain=noise(uv*90.0+vec2(m.finish.y));
    var base=color;
    var spec=0.0;
    var lighting=0.32+0.68*diffuse;
    if kind==MATERIAL_CERAMIC {
        spec=pow(max(dot(n,half_dir),0.0),mix(120.0,12.0,m.params.y))*m.finish.x;
        base*=1.0+(noise(uv*7.0)-0.5)*m.params.z*0.1;
    }
    if kind==MATERIAL_BRUSHED_METAL || kind==MATERIAL_SATIN_METAL {
        // Size-dependent frequency is filtered before pixel-scale aliasing.
        let frequency=min(450.0*m.params.w,pixels*0.4);
        let streak=noise(vec2(brush.x*frequency,brush.y));
        base*=0.95+(streak-0.5)*m.params.z*select(0.16,0.035,kind==MATERIAL_SATIN_METAL);
        let nh=max(dot(n,half_dir),0.0);
        let th=dot(tangent,half_dir);
        let bitangent=normalize(cross(n,tangent)+vec3(0.00001,0.0,0.0));
        let bh=dot(bitangent,half_dir);
        let stretch=mix(0.15,0.35,m.params.y);
        spec=exp(-(th*th/(stretch*stretch)+bh*bh/0.65)/max(nh*nh,0.01))*m.finish.x*0.8;
        lighting=0.45+0.55*diffuse;
    }
    if kind==MATERIAL_PAPER {
        let q=vec3(uv,p.z*m.params.w)+vec3(m.finish.y);
        let resolution=pixels*max(abs(dot(n,view)),0.08)/m.params.w;
        let paper_grain=material_band(q,vec3(90.0),resolution);
        let fibres=material_band(q,vec3(32.0,90.0,80.0),resolution);
        let pulp=material_band(q,vec3(22.0),resolution);
        base*=1.0+m.params.z*(paper_grain*0.14+fibres*0.045+pulp*0.012);
        lighting=0.8+0.2*diffuse;
    }
    if kind==MATERIAL_WOOD {
        // Unpatterned, rough plywood: irregular fine fibres, not decorative
        // annual rings. Object-space z varies the grain along the wall
        // height and shares the same surface treatment with the top face.
        let q=vec3(uv,p.z*m.params.w)+vec3(m.finish.y);
        let resolution=pixels*max(abs(dot(n,view)),0.08)/m.params.w;
        let fibres=material_band(q,vec3(72.0,85.0,90.0),resolution);
        let fine=material_band(q,vec3(150.0,125.0,135.0),resolution);
        let pores=material_band(q,vec3(22.0,28.0,24.0),resolution);
        base*=1.0+m.params.z*(fibres*0.22+fine*0.10+pores*0.04);
        lighting=0.7+0.3*diffuse;
        spec=pow(max(dot(n,half_dir),0.0),mix(48.0,8.0,m.params.y))*m.finish.x*0.025*(1.0-m.params.y);
    }
    if kind==MATERIAL_TOON { lighting=select(select(0.5,0.76,diffuse>0.3),1.0,diffuse>0.7); }
    if kind==MATERIAL_ENAMEL {
        spec=pow(max(dot(n,half_dir),0.0),mix(180.0,22.0,m.params.y))*m.finish.x;
        lighting=0.65+0.35*diffuse;
    }
    if kind==MATERIAL_HATCH {
        let frequency=min(45.0*m.params.w,pixels*0.15);
        let hatch=smoothstep(0.15,0.4,abs(sin((uv.x+uv.y)*frequency)));
        base*=1.0-(1.0-hatch)*m.params.z*0.35;
    }
    if kind==MATERIAL_PEARL {
        let fresnel=pow(1.0-max(dot(n,view),0.0),3.0);
        let pearl=0.5+0.5*cos(vec3(0.0,2.1,4.2)+dot(n,view)*9.0);
        base=mix(base,pearl,fresnel*m.params.z*0.25);
        spec=pow(max(dot(n,half_dir),0.0),32.0)*m.finish.x;
    }
    if kind==MATERIAL_MATTE { base*=1.0+(grain-0.5)*m.params.z*0.08; }
    let metal=kind==MATERIAL_BRUSHED_METAL || kind==MATERIAL_SATIN_METAL;
    return clamp(base*lighting+vec3(m.finish.w)+mix(vec3(spec),base*spec,select(0.0,0.7,metal)),vec3(0.0),vec3(1.0));
}
