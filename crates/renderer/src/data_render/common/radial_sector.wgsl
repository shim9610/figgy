// The radial sector: its storage layout and the ray-marched solid that the
// radial draw shades and the radial pick picks (SHADER_COMMON.md §5.12).
// Spliced after `material.wgsl` into the draw module (`radial.wgsl`) and the
// pick module (`radial_pick.wgsl`), so both march the same sector.
struct Sector {
    geometry: vec4<f32>, // center.xy, radius, inner radius
    arc: vec4<f32>, // start, span, tilt, depth
    color: vec4<f32>,
    material: Material, // SHADER_COMMON.md §5.9
    detail: vec4<f32>, // explode, angular gap, bevel, reserved (0)
    rounding: vec4<f32>, // inner corner, outer corner, vertical lift, reserved (0)
    outline: vec4<f32>, // rim, separators, emphasis amount, physical width
    outline_color: vec4<f32>,
    // reserved (0), reserved (0), inside-label flag, inside-label lift: how far
    // the lifted top face moves up the screen in physical pixels, computed once
    // on the CPU where the sector is built (radial_layout.rs); the moving
    // inside glyphs are sampled that far below the pixel
    effects: vec4<f32>,
    light: vec4<f32>, // direction.xyz, shadow enabled
};
const TAU: f32 = 6.28318530718;
struct Hit { t: f32, p: vec3<f32>, n: vec3<f32> };
// Rounded intersections round the actual sector footprint, including its walls.
// The pick module picks with these same functions; the CPU twin in
// tests/support/cpu_twin.rs follows them for the pick agreement test.
fn round_intersection(a: f32,b: f32,r: f32) -> f32 {
    if r < 0.000001 { return max(a,b); }
    return min(-r,max(a,b))+length(max(vec2(r+a,r+b),vec2(0.0)));
}
fn edges(p: vec2<f32>,s: Sector) -> vec3<f32> {
    let r=length(p);
    if s.arc.y>=TAU-0.00001 { return vec3(r-1.0,select(-10.0,s.geometry.w-r,s.geometry.w>0.0),-10.0); }
    let a=s.arc.x+s.detail.y*0.5;
    let b=s.arc.x+s.arc.y-s.detail.y*0.5;
    let da=dot(p,vec2(sin(a),-cos(a)));
    let db=dot(p,vec2(-sin(b),cos(b)));
    return vec3(r-1.0,select(-10.0,s.geometry.w-r,s.geometry.w>0.0),select(min(da,db),max(da,db),b-a<=TAU*0.5));
}
fn footprint(p: vec2<f32>,s: Sector) -> f32 {
    var d=edges(p,s);
    if s.arc.y>=TAU-0.00001 { return max(d.x,d.y); }
    let cap=min((1.0-s.geometry.w)*0.45,(s.arc.y-s.detail.y)*max(s.geometry.w,0.2)*0.35);
    if s.geometry.w==0.0 && s.arc.y-s.detail.y<=TAU*0.5 {
        let a=s.arc.x+s.detail.y*0.5;let b=s.arc.x+s.arc.y-s.detail.y*0.5;
        d.z=round_intersection(dot(p,vec2(sin(a),-cos(a))),dot(p,vec2(-sin(b),cos(b))),min(s.rounding.x,cap));
    }
    return max(round_intersection(d.x,d.z,min(s.rounding.y,cap)),round_intersection(d.y,d.z,min(s.rounding.x,cap)));
}
fn solid(p: vec3<f32>,s: Sector) -> f32 {
    let bevel=min(min(s.detail.z,s.arc.w*0.48),(1.0-s.geometry.w)*0.2);
    let q=vec2(footprint(p.xy,s)+bevel,abs(p.z-s.arc.w*0.5)-(s.arc.w*0.5-bevel));
    return length(max(q,vec2(0.0)))+min(max(q.x,q.y),0.0)-bevel;
}
fn hit_sector(ro: vec3<f32>,rd: vec3<f32>,s: Sector) -> Hit {
    var t=max((s.arc.w-ro.z)/rd.z,0.0);
    let end=-ro.z/rd.z+0.0002;
    if s.arc.w<=0.0 {
        let p=ro+rd*t;
        if footprint(p.xy,s)<=0.0 { return Hit(t,p,vec3(0.0,0.0,1.0)); }
        return Hit(10000.0,vec3(0.0),vec3(0.0));
    }
    for(var step=0u;step<128u;step++) {
        if t>end { break; }
        let p=ro+rd*t;let d=solid(p,s);
        if d<0.00012 {
            let e=0.0002;
            let n=vec3(solid(p+vec3(e,0.0,0.0),s)-solid(p-vec3(e,0.0,0.0),s),
                solid(p+vec3(0.0,e,0.0),s)-solid(p-vec3(0.0,e,0.0),s),
                solid(p+vec3(0.0,0.0,e),s)-solid(p-vec3(0.0,0.0,e),s));
            return Hit(t,p,normalize(n+vec3(0.0,0.0,0.00000001)));
        }
        t+=max(d,0.00005);
    }
    return Hit(10000.0,vec3(0.0),vec3(0.0));
}
// The nearest hit so far, kept as one value: the order key of its `t` (the
// bits with the sign bit cleared, so a `-0.0` start orders as 0; `t` is
// never negative, and the bits of a float that is not negative order as its
// value does) and its sector. Metal drops the winner of a loop that updates
// the distance and the index as two variables, keeping the nearest `t` with
// sector 0 (#63); one `vec2<u32>` assignment moves both together.
fn nearest_hit(best: vec2<u32>,t: f32,i: u32) -> vec2<u32> {
    let key=bitcast<u32>(t)&0x7fffffffu;
    return select(best,vec2(key,i),key<best.x);
}
const NO_HIT: vec2<u32> = vec2(0x461c4000u,0u); // nearest_hit's key of t = 10000
fn hit_pixel(pixel: vec2<f32>,s: Sector) -> Hit {
    let cs=cos(s.arc.z);let sn=sin(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
    let center=s.geometry.xy+s.detail.x*s.geometry.z*vec2(cos(mid),sin(mid)*cs);
    let xy=(pixel-center)/s.geometry.z;
    if abs(xy.x)>1.001 || abs(xy.y)>1.0+s.arc.w+s.rounding.z {
        return Hit(10000.0,vec3(0.0),vec3(0.0));
    }
    return hit_sector(vec3(xy.x,xy.y*cs+sn*3.0,-xy.y*sn+cs*3.0-s.rounding.z),vec3(0.0,-sn,-cs),s);
}
