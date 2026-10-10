// Independent radial geometry in screen/radius coordinates. No Cartesian
// Transform/Style blocks from SHADER_COMMON.md are used here.
struct Globals {
    size: vec4<f32>, // width, height, slice count, AA grid side (2 or 3)
    background: vec4<f32>,
    light: vec4<f32>,
    shadow: vec4<f32>,
    groups: array<vec4<f32>, 2>, // center.xy, radius, cos(tilt)
    group_info: vec4<f32>, // group count, output sRGB, inner radii
};
struct Sector {
    geometry: vec4<f32>, // center.xy, radius, inner radius
    arc: vec4<f32>, // start, span, tilt, depth
    color: vec4<f32>,
    material: vec4<f32>, // kind, roughness, texture strength, frequency
    detail: vec4<f32>, // explode, angular gap, bevel, seed
    rounding: vec4<f32>, // inner corner, outer corner, vertical lift, gloss
    outline: vec4<f32>, // rim, separators, emphasis amount, physical width
    outline_color: vec4<f32>,
    effects: vec4<f32>, // highlight brightness, texture angle, reserved
    light: vec4<f32>, // direction.xyz, shadow enabled
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> sectors: array<Sector>;
@group(0) @binding(2) var annotations: texture_2d<f32>;
const TAU: f32 = 6.28318530718;
struct Hit { t: f32, p: vec3<f32>, n: vec3<f32> };
// Rounded intersections round the actual sector footprint, including its walls.
// Keep these equations synchronized with radial_geometry.rs (pixel/picking test).
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
fn hash(p: vec2<f32>) -> f32 { return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
fn noise(p: vec2<f32>) -> f32 {
    let i=floor(p);let f=fract(p);let u=f*f*(3.0-2.0*f);
    return mix(mix(hash(i),hash(i+vec2(1.0,0.0)),u.x),mix(hash(i+vec2(0.0,1.0)),hash(i+vec2(1.0)),u.x),u.y);
}
// Continuous solid noise: paper must vary in z on an extruded wall, not
// repeat a top-plane sample down its entire height. No face UV seams.
fn paper_noise(p: vec3<f32>) -> f32 {
    let z=floor(p.z);let f=fract(p.z);let blend=f*f*(3.0-2.0*f);
    return mix(noise(p.xy+z*vec2(37.0,91.0)),noise(p.xy+(z+1.0)*vec2(37.0,91.0)),blend);
}
fn paper_band(p: vec3<f32>,frequency: vec3<f32>,pixels: f32) -> f32 {
    // Keep coordinates fixed in object space. Fade frequencies that cannot be
    // resolved at this output size/angle instead of changing the pattern scale.
    let cycles=max(max(frequency.x,frequency.y),frequency.z)/max(pixels,1.0);
    let resolved=1.0-smoothstep(0.3,0.85,cycles);
    return (paper_noise(p*frequency)-0.5)*resolved;
}
fn surface(s: Sector,h: Hit,view: vec3<f32>) -> vec3<f32> {
    let kind=u32(s.material.x);
    if kind==0u { return min(s.color.rgb+vec3(s.effects.x),vec3(1.0)); }
    let l=normalize(s.light.xyz);let half_dir=normalize(l+view);
    let diffuse=max(dot(h.n,l),0.0);
    let ca=cos(s.effects.y);let sa=sin(s.effects.y);
    let p=vec2(h.p.x*ca-h.p.y*sa,h.p.x*sa+h.p.y*ca)*s.material.w;
    let grain=noise(p*90.0+vec2(s.detail.w));
    var base=s.color.rgb;
    var spec=0.0;
    var lighting=0.32+0.68*diffuse;
    if kind==2u {
        spec=pow(max(dot(h.n,half_dir),0.0),mix(120.0,12.0,s.material.y))*s.rounding.w;
        base*=1.0+(noise(p*7.0)-0.5)*s.material.z*0.1;
    }
    if kind==3u || kind==6u {
        // Radius-dependent frequency is filtered before pixel-scale aliasing.
        let frequency=min(450.0*s.material.w,s.geometry.z*0.4);
        let brush=noise(vec2(length(h.p.xy)*frequency,atan2(p.y,p.x)*2.0));
        base*=0.95+(brush-0.5)*s.material.z*select(0.16,0.035,kind==6u);
        let direction=vec3(-h.p.y*ca-h.p.x*sa,-h.p.y*sa+h.p.x*ca,0.0);
        let tangent=normalize(direction-h.n*dot(h.n,direction)+vec3(0.000001,0.0,0.0));
        let nh=max(dot(h.n,half_dir),0.0);
        let th=dot(tangent,half_dir);
        let bitangent=normalize(cross(h.n,tangent)+vec3(0.00001,0.0,0.0));
        let bh=dot(bitangent,half_dir);
        let stretch=mix(0.15,0.35,s.material.y);
        spec=exp(-(th*th/(stretch*stretch)+bh*bh/0.65)/max(nh*nh,0.01))*s.rounding.w*0.8;
        lighting=0.45+0.55*diffuse;
    }
    if kind==4u {
        let q=vec3(p,h.p.z*s.material.w)+vec3(s.detail.w);
        let pixels=s.geometry.z*max(abs(dot(h.n,view)),0.08)/s.material.w;
        let paper_grain=paper_band(q,vec3(90.0),pixels);
        let fibres=paper_band(q,vec3(32.0,90.0,80.0),pixels);
        let pulp=paper_band(q,vec3(22.0),pixels);
        base*=1.0+s.material.z*(paper_grain*0.14+fibres*0.045+pulp*0.012);
        lighting=0.8+0.2*diffuse;
    }
    if kind==5u {
        let rings=sin((p.x*28.0+noise(p*3.0)*4.0+sin(p.y*4.0))*3.0);
        base*=1.0+s.material.z*(rings*0.24+(grain-0.5)*0.2);
        spec=pow(max(dot(h.n,half_dir),0.0),28.0)*0.15;
    }
    if kind==7u { lighting=select(select(0.5,0.76,diffuse>0.3),1.0,diffuse>0.7); }
    if kind==8u {
        spec=pow(max(dot(h.n,half_dir),0.0),mix(180.0,22.0,s.material.y))*s.rounding.w;
        lighting=0.65+0.35*diffuse;
    }
    if kind==9u {
        let frequency=min(45.0*s.material.w,s.geometry.z*0.15);
        let hatch=smoothstep(0.15,0.4,abs(sin((p.x+p.y)*frequency)));
        base*=1.0-(1.0-hatch)*s.material.z*0.35;
    }
    if kind==10u {
        let fresnel=pow(1.0-max(dot(h.n,view),0.0),3.0);
        let pearl=0.5+0.5*cos(vec3(0.0,2.1,4.2)+dot(h.n,view)*9.0);
        base=mix(base,pearl,fresnel*s.material.z*0.25);
        spec=pow(max(dot(h.n,half_dir),0.0),32.0)*s.rounding.w;
    }
    if kind==1u { base*=1.0+(grain-0.5)*s.material.z*0.08; }
    return clamp(base*lighting+vec3(s.effects.x)+mix(vec3(spec),base*spec,select(0.0,0.7,kind==3u || kind==6u)),vec3(0.0),vec3(1.0));
}
fn hit_pixel(pixel: vec2<f32>,s: Sector) -> Hit {
    let cs=cos(s.arc.z);let sn=sin(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
    let center=s.geometry.xy+s.detail.x*s.geometry.z*vec2(cos(mid),sin(mid)*cs);
    let xy=(pixel-center)/s.geometry.z;
    if abs(xy.x)>1.001 || abs(xy.y)>1.0+s.arc.w+s.rounding.z {
        return Hit(10000.0,vec3(0.0),vec3(0.0));
    }
    return hit_sector(vec3(xy.x,xy.y*cs+sn*3.0,-xy.y*sn+cs*3.0-s.rounding.z),vec3(0.0,-sn,-cs),s);
}
fn scene(pixel: vec2<f32>) -> vec4<f32> {
    var color=vec4(globals.background.rgb*globals.background.a,globals.background.a);
    // Per-slice floor shadows follow lift without changing the chart bounds.
    var shade=0.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        if s.light.w<=0.0 { continue; }
        let cs=cos(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
        let elevation=s.arc.w+s.rounding.z;
        let offset=vec2(0.05,0.08)+normalize(-s.light.xy+vec2(0.00001))*elevation*0.3;
        let xy=(pixel-s.geometry.xy)/s.geometry.z-vec2(cos(mid),sin(mid)*cs)*s.detail.x-offset;
        let d=footprint(vec2(xy.x,xy.y/cs),s);
        let blur=0.025+elevation*0.13;
        shade=max(shade,(1.0-smoothstep(-blur,blur*2.0,d))*0.16);
    }
    color=vec4(color.rgb*(1.0-shade),shade+color.a*(1.0-shade));
    var closest=10000.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        let cs=cos(s.arc.z);let sn=sin(s.arc.z);
        let rd=vec3(0.0,-sn,-cs);
        let h=hit_pixel(pixel,s);
        if h.t<closest {
            closest=h.t;
            var rgb=surface(s,h,-rd);
            let d=edges(h.p.xy,s);
            let rim_distance=min(abs(d.x),abs(d.y));
            let separation=abs(d.z);
            let width=s.outline.w/s.geometry.z;
            let is_rim=rim_distance<=separation;
            let enabled=select(s.outline.y,s.outline.x,is_rim);
            if max(enabled,s.outline.z)>0.0 && width>0.0 {
                let band=s.outline.w*select(0.5,1.0,is_rim);
                let offsets=array<vec2<f32>,4>(vec2(band,0.0),vec2(-band,0.0),vec2(0.0,band),vec2(0.0,-band));
                var border=false;
                for(var j=0u;j<4u;j++) { border=border || hit_pixel(pixel+offsets[j],s).t>=9999.0; }
                if border { rgb=mix(rgb,s.outline_color.rgb,s.outline_color.a*max(enabled,s.outline.z)); }
            }
            color=vec4(rgb,1.0);
        }
    }
    return color;
}
// Outside labels stay fixed during interaction. Inside glyphs translate with
// their slice by sampling the unchanged full-resolution annotation texture.
fn label_pixel(pixel: vec2<f32>) -> vec4<f32> {
    var text=textureLoad(annotations,vec2<i32>(pixel),0);
    var moving=false;
    for(var i=0u;i<u32(globals.size.z);i++) {
        var s=sectors[i];
        if s.effects.z>0.0 && s.rounding.z>0.0 {
            moving=true;s.rounding.z=0.0;
            let rest=hit_pixel(pixel,s);
            if rest.t<9999.0 && rest.n.z>0.5 { text=vec4(0.0); }
        }
    }
    if !moving { return text; }
    var closest=10000.0;var winner=0u;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let h=hit_pixel(pixel,sectors[i]);
        if h.t<closest {closest=h.t;winner=i;}
    }
    if closest<9999.0 {
        let s=sectors[winner];
        if s.effects.z>0.0 && s.rounding.z>0.0 {
            let offset=vec2(0.0,s.rounding.z*sin(s.arc.z)*s.geometry.z);
            let location=pixel+offset;
            let lo=vec2<i32>(floor(location-0.5));let f=fract(location-0.5);
            // Subpixel translation only, no rescaling or repeated texture feedback.
            text=mix(mix(textureLoad(annotations,lo,0),textureLoad(annotations,lo+vec2(1,0),0),f.x),
                     mix(textureLoad(annotations,lo+vec2(0,1),0),textureLoad(annotations,lo+vec2(1,1),0),f.x),f.y);
        }
    }
    return text;
}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0));return vec4(p[i],0.0,1.0);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    var color=vec4(0.0);let grid=u32(globals.size.w);
    for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
        color+=scene(p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5);
    }}
    color/=f32(grid*grid);
    let text=label_pixel(p.xy);
    color=text+color*(1.0-text.a);
    if globals.group_info.y>0.0 && color.a>0.0 {
        let c=color.rgb/color.a;
        let linear=select(pow((c+0.055)/1.055,vec3(2.4)),c/12.92,c<=vec3(0.04045));
        color=vec4(linear*color.a,color.a);
    }
    return color;
}
