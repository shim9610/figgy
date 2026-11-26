// Independent radial geometry in screen/radius coordinates. No Cartesian
// Transform/Style fragments are used here.
struct Globals {
    size: vec4<f32>, // width, height, slice count, AA grid side (2 or 3)
    background: vec4<f32>,
    light: vec4<f32>,
    shadow: vec4<f32>,
    groups: array<vec4<f32>, 2>, // center.xy, radius, cos(tilt)
    group_info: vec4<f32>, // group count, output sRGB, inner radii
};
// [data_render/common/radial_sector.wgsl: struct Sector]
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> sectors: array<Sector>;
@group(0) @binding(2) var annotations: texture_2d<f32>;
// [data_render/common/radial_sector.wgsl: TAU, Hit and the sector ray-march, round_intersection to hit_pixel]
fn crosses_top_wall(a: Hit,b: Hit) -> bool {
    // Classify the visible geometric surface, including the middle of a bevel.
    // Testing first hits avoids drawing hidden edges through the solid.
    return b.t<9999.0 && (a.n.z>=0.70710678)!=(b.n.z>=0.70710678);
}
// The floor shadow under `pixel`: a soft blur, taken once per pixel (#349).
// Per-slice floor shadows follow lift without changing the chart bounds.
fn floor_shadow(pixel: vec2<f32>) -> f32 {
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
    return shade;
}
// What one antialiasing sample covers: the nearest sector, the first of
// equals as in the radial pick, and its hit. `winner` counts sectors from
// 1, and 0 means the background: Metal compiles a 0-based `winner=i` as a
// constant 0 (#63).
struct Coverage { winner: u32, hit: Hit };
fn coverage(pixel: vec2<f32>) -> Coverage {
    var closest=10000.0;var winner=0u;var h=Hit(10000.0,vec3(0.0),vec3(0.0));
    for(var i=0u;i<u32(globals.size.z);i++) {
        let hit=hit_pixel(pixel,sectors[i]);
        if hit.t<closest {closest=hit.t;winner=i+1u;h=hit;}
    }
    return Coverage(winner,h);
}
// The lit material colour of sector `s` where `h` hits it.
fn sector_surface(h: Hit,s: Sector) -> vec3<f32> {
    let cs=cos(s.arc.z);let sn=sin(s.arc.z);
    let rd=vec3(0.0,-sn,-cs);
    // The brush of BrushedMetal and SatinMetal runs around the
    // center: its tangent is circumferential in the turned texture
    // frame, its coordinates are (radius, twice the turned angle).
    let ca=cos(s.material.finish.z);let sa=sin(s.material.finish.z);
    let turned=vec2(h.p.x*ca-h.p.y*sa,h.p.x*sa+h.p.y*ca)*s.material.params.w;
    let direction=vec3(-h.p.y*ca-h.p.x*sa,-h.p.y*sa+h.p.x*ca,0.0);
    let tangent=normalize(direction-h.n*dot(h.n,direction)+vec3(0.000001,0.0,0.0));
    return material_surface(s.material,s.color.rgb,h.p,h.n,-rd,normalize(s.light.xyz),tangent,
        vec2(length(h.p.xy),atan2(turned.y,turned.x)*2.0),s.geometry.z);
}
// `surface` inked with the outline of sector `s` where the sample at `pixel`
// hits it at `h`. Taken per sample, so the ink keeps its antialiasing.
fn outlined(pixel: vec2<f32>,h: Hit,s: Sector,surface: vec3<f32>) -> vec3<f32> {
    var rgb=surface;
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
        for(var j=0u;j<4u;j++) {
            let neighbor=hit_pixel(pixel+offsets[j],s);
            border=neighbor.t>=9999.0;
            if !border && crosses_top_wall(h,neighbor) {
                // A crease is inked on both faces, unlike the inward
                // silhouette band. Keep the total logical width equal.
                if is_rim {
                    border=crosses_top_wall(h,hit_pixel(pixel+offsets[j]*0.5,s));
                } else { border=true; }
            }
            if border { break; }
        }
        if border { rgb=mix(rgb,s.outline_color.rgb,s.outline_color.a*max(enabled,s.outline.z)); }
    }
    return rgb;
}
// Flat, level and unlit sectors (#41). At depth and tilt zero a sector is hit
// exactly where its footprint is at most zero, at a depth that is the same at
// every pixel, in a Flat colour that does not depend on where it is hit, and
// its normal is straight up, so `outlined` inks a hit exactly where one of
// the four neighbours `band` away along x or y misses the sector: where the
// largest of their footprints is above zero. Both change by at most sqrt(2)
// times the distance moved, so when every sector's footprint and ink boundary
// at the pixel center are at least `margin` pixels from zero, every
// antialiasing sample lies on the same side of each and the sampled path
// returns one colour for each of them: the nearest covering sector's, inked
// or not. Nearer an edge the pixel takes analytic coverage instead (no
// supersampling): each boundary's distance in physical pixels smoothed over
// the footprint's `fwidth`,
// so a slanted edge, a thin angular gap or a separator ramps through every
// value rather than the quarter steps of the 2 x 2 grid. Adjacent sectors
// (the rings of a sunburst, slices without a gap) measure complementary
// distances, so their coverages add to one across a shared edge and no
// background shows through. Only how an edge is antialiased depends on this:
// which sector covers a point, its colour and its ink are as on the sampled
// path. `exact` is false when a sector needs another feature, or for a NaN
// distance, and the samples are taken.
struct FlatPixel { exact: bool, color: vec4<f32> };
fn flat_pixel(pixel: vec2<f32>,margin: f32) -> FlatPixel {
    let background=vec4(globals.background.rgb*globals.background.a,globals.background.a);
    var color=background;
    var closest=10000.0;
    var near=false;var finite=true;
    var covered=vec3(0.0);var total=0.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        // Uniform: the sectors are read-only storage, so leaving here keeps
        // the derivatives below in uniform control flow.
        if u32(s.material.params.x)!=MATERIAL_FLAT || s.arc.w>0.0 || s.arc.z!=0.0 || s.light.w>0.0 {
            return FlatPixel(false,color);
        }
        let mid=s.arc.x+s.arc.y*0.5;
        let center=s.geometry.xy+s.detail.x*s.geometry.z*vec2(cos(mid),sin(mid));
        let xy=(pixel-center)/s.geometry.z;
        let d=footprint(xy,s)*s.geometry.z;
        // Taken for every sector before any choice made per pixel.
        // The footprint is a distance: its gradient has unit length, so its
        // `fwidth` lies between 1 and sqrt(2) on an edge. Across a crease (a
        // pie's apex, where two edges meet) the quad's differences cancel,
        // so the width keeps at least 1; at most 1.5 keeps its smoothing
        // inside `margin`.
        let aa=clamp(fwidth(d),1.0,1.5);
        let flat=min(s.color.rgb+vec3(s.material.finish.w),vec3(1.0));
        // `outlined` at a level Flat hit: the rim's band or half the
        // separators', the hit inked where one of its four neighbours that
        // far along x or y misses the sector. `ink` is the largest of their
        // footprints, in physical pixels: above zero where the hit is inked.
        // A neighbour is at most `band` away, so where the footprint is below
        // minus `band` and `margin` none can miss.
        let e=edges(xy,s);
        let is_rim=min(abs(e.x),abs(e.y))<=abs(e.z);
        let amount=max(select(s.outline.y,s.outline.x,is_rim),s.outline.z);
        let inks=amount>0.0 && s.outline.w>0.0;
        let band=s.outline.w*select(0.5,1.0,is_rim);
        var ink=d+band;
        if inks && d+band>-margin && d<margin {
            ink=-10000.0;
            let offsets=array<vec2<f32>,4>(vec2(band,0.0),vec2(-band,0.0),vec2(0.0,band),vec2(0.0,-band));
            for(var j=0u;j<4u;j++) {
                ink=max(ink,footprint((pixel+offsets[j]-center)/s.geometry.z,s)*s.geometry.z);
            }
        }
        let inked=select(flat,mix(flat,s.outline_color.rgb,s.outline_color.a*amount),inks);
        // A NaN distance is neither.
        let edge=min(abs(d),select(10000.0,abs(ink),inks));
        near=near || edge<margin;
        finite=finite && (edge>=margin || edge<margin);
        let a=1.0-smoothstep(-0.5*aa,0.5*aa,d);
        // The inked band's inner boundary, smoothed over the footprint's own
        // width: both are distances, moved together along an edge.
        let plain=select(a,1.0-smoothstep(-0.5*aa,0.5*aa,ink),inks);
        // Ink adds its difference from the fill over its band, so an ink
        // that changes no colour (a transparent one) changes no bit.
        covered+=flat*a+(inked-flat)*(a-plain);total+=a;
        // `hit_sector`'s depth at zero tilt: the ray starts at 3 - lift.
        let t=max((s.arc.w-(3.0-s.rounding.z))/-1.0,0.0);
        if d<=0.0 && t<closest {
            closest=t;
            color=vec4(select(flat,inked,inks && ink>0.0),1.0);
        }
    }
    if !finite { return FlatPixel(false,color); }
    if near {
        // Overlapping coverage (more than one in all) shares the pixel.
        let share=min(total,1.0);
        // Premultiplied: the sectors are opaque, so `covered` already is.
        color=vec4(covered/max(total,1.0)+background.rgb*(1.0-share),share+background.a*(1.0-share));
    }
    return FlatPixel(true,color);
}
// Whether `flat_pixel` takes the chart: every sector Flat, level and
// unshadowed (or no sector at all). Uniform: it reads read-only storage.
fn flat_chart() -> bool {
    // Field by field: a whole sector per pixel and sector cost a Flat chart
    // a third more on SwiftShader.
    for(var i=0u;i<u32(globals.size.z);i++) {
        if u32(sectors[i].material.params.x)!=MATERIAL_FLAT || sectors[i].arc.w>0.0
            || sectors[i].arc.z!=0.0 || sectors[i].light.w>0.0 {
            return false;
        }
    }
    return true;
}
// Where the ray `hit_pixel` casts through `pixel` meets the plane of sector
// `s`'s top face, in the sector's frame: x and y on the footprint; z, how far
// y falls while the ray crosses the solid's depth (inside the solid the ray
// runs from y down to y - z at this x); w, the ray's height above the top
// where it starts, so that t at the top is w / cos(tilt).
fn top_plane(pixel: vec2<f32>,s: Sector) -> vec4<f32> {
    let cs=cos(s.arc.z);let sn=sin(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
    let center=s.geometry.xy+s.detail.x*s.geometry.z*vec2(cos(mid),sin(mid)*cs);
    let xy=(pixel-center)/s.geometry.z;
    return vec4(xy.x,(xy.y+sn*(s.rounding.z+s.arc.w))/cs,sn*max(s.arc.w,0.0)/cs,
        -xy.y*sn+cs*3.0-s.rounding.z-s.arc.w);
}
// The hit `hit_sector` returns on the flat part of a top face, at the ray's
// point on the top plane under `pixel`, whether or not the sector covers it:
// what an edge pixel shades a sector with where its first sample misses it.
fn top_hit(pixel: vec2<f32>,s: Sector) -> Hit {
    let cs=cos(s.arc.z);let sn=sin(s.arc.z);let mid=s.arc.x+s.arc.y*0.5;
    let center=s.geometry.xy+s.detail.x*s.geometry.z*vec2(cos(mid),sin(mid)*cs);
    let xy=(pixel-center)/s.geometry.z;
    let ro=vec3(xy.x,xy.y*cs+sn*3.0,-xy.y*sn+cs*3.0-s.rounding.z);let rd=vec3(0.0,-sn,-cs);
    let t=max((s.arc.w-ro.z)/rd.z,0.0);
    return Hit(t,ro+rd*t,vec3(0.0,0.0,1.0));
}
// A lower bound of `footprint` along the segment x = `x`, y from `y0` to
// `y1`: the footprint is at least its unrounded edges (`round_intersection`
// of a and b is at least max(a, b)), and the least value of each edge on the
// segment is exact. The rings' distances are least where y is nearest 0 or
// farthest from it; the angular edges are linear in y, their larger (a span
// up to half a turn) convex, least at an end or where they cross.
fn column_floor(x: f32,y0: f32,y1: f32,s: Sector) -> f32 {
    let far=max(length(vec2(x,y0)),length(vec2(x,y1)));
    let rings=max(length(vec2(x,clamp(0.0,y0,y1)))-1.0,select(-10.0,s.geometry.w-far,s.geometry.w>0.0));
    if s.arc.y>=TAU-0.00001 { return rings; }
    let a=s.arc.x+s.detail.y*0.5;let b=s.arc.x+s.arc.y-s.detail.y*0.5;
    let na=vec2(sin(a),-cos(a));let nb=vec2(-sin(b),cos(b));
    let a0=dot(vec2(x,y0),na);let a1=dot(vec2(x,y1),na);
    let b0=dot(vec2(x,y0),nb);let b1=dot(vec2(x,y1),nb);
    var side=min(min(a0,a1),min(b0,b1));
    if b-a<=TAU*0.5 {
        side=min(max(a0,b0),max(a1,b1));
        let slope=nb.y-na.y;
        if abs(slope)>0.000001 {
            let y=clamp(x*(na.x-nb.x)/slope,y0,y1);
            side=min(side,max(dot(vec2(x,y),na),dot(vec2(x,y),nb)));
        }
    }
    return max(rings,side);
}
// Whether every sector's coverage is its top face's footprint: no depth
// (`hit_sector` takes the plane), or level with no bevel, where a straight
// ray down meets the flat top or nothing. Uniform: it reads read-only storage.
fn analytic_chart() -> bool {
    for(var i=0u;i<u32(globals.size.z);i++) {
        let arc=sectors[i].arc;
        let bevel=min(min(sectors[i].detail.z,arc.w*0.48),(1.0-sectors[i].geometry.w)*0.2);
        if arc.w>0.0 && (arc.z!=0.0 || bevel>0.0) { return false; }
    }
    return true;
}
// A chart `flat_pixel` does not take: materials, tilt, depth, bevel or a
// shadow (#454). One ray per pixel replaces the samples wherever that gives
// the samples' own colour or an analytic edge; `exact` is false where it
// cannot, and the pixel takes the samples. `color` is premultiplied and
// leaves out the background: the caller adds the ground (the background and
// its floor shadow) times one minus its alpha, so `floor_shadow` is inlined
// once.
//
// Each sector is measured at the pixel centre on its top plane
// (`top_plane`). The footprint is a distance on that plane, and a pixel's
// samples, within 0.48 px of its centre, move it by at most `reach`: `margin`
// over the radius times cos(tilt), since tilt shortens y on screen. A sector
// is out of every sample's reach when the footprint stays above zero along
// the whole column its rays cross inside the solid (`column_floor`, less
// `reach`): `hit_sector` stops only where `solid`, at least the footprint,
// is below 0.00012.
//
// - No sector in reach: the background, as every sample draws it.
// - Every sample on one sector's flat top (the footprint below minus the
//   bevel, `reach` and the outline band), and every other sector out of
//   reach or nearer the ground than that top along each ray: every sample
//   hits that sector there, straight up, so they share one normal, one
//   material evaluated at the first sample (what the samples share) and no
//   ink. The pixel takes that colour from one ray at the first sample.
// - A chart whose coverage is its top faces' footprints (`analytic_chart`),
//   with every sector's ray starting above its top: `flat_pixel`'s analytic
//   edges, over each sector's footprint on its top plane (in physical pixels
//   along x and up to 1 / cos(tilt) times that along y, so `fwidth` is
//   clamped to [1, 1.5 / cos(tilt)]), each sector shaded once at the first
//   sample (`hit_pixel`, or `top_hit` where that sample misses it) and the
//   background with its floor shadow. A pixel at least `margin` from every
//   edge takes the nearest covering sector's colour, as its samples do.
// - Anything else takes the samples: a bevel or a crease (normals that differ
//   across the pixel), a tilted side wall (where it meets its top face or the
//   background, the samples shade the two apart), sectors that may overlap,
//   a NaN distance.
//
// Only how a pixel is antialiased depends on this: which sector covers a
// point, its colour and its ink are as on the sampled path. The derivatives
// are taken for every sector before any choice made per pixel, so they stay
// in uniform control flow.
fn material_pixel(pixel: vec2<f32>,margin: f32) -> FlatPixel {
    let first=pixel+(vec2(0.0)+0.5)/f32(u32(globals.size.w))-0.5;
    let analytic=analytic_chart();
    var color=vec4(0.0);var closest=10000.0;
    var near=false;var finite=true;var rays=true;
    var covered=vec3(0.0);var total=0.0;
    // Whether any sector is in reach; how many sectors have a flat top that
    // every sample hits, the colour of that top and the largest t there; the
    // least t at which any other sector in reach can be hit.
    var reached=false;var tops=0u;var shaded=vec3(0.0);var top_t=0.0;var other=10000.0;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let s=sectors[i];
        let cs=cos(s.arc.z);let sn=sin(s.arc.z);
        let c=top_plane(pixel,s);
        let f=footprint(c.xy,s);
        let d=f*s.geometry.z;
        // Taken for every sector before any choice made per pixel. Across a
        // crease the quad's differences cancel, so the width keeps at least 1.
        let aa=clamp(fwidth(d),1.0,1.5/cs);
        let reach=margin/(s.geometry.z*cs);
        finite=finite && (f>=0.0 || f<0.0);
        var out=f-c.z-reach>0.001;
        if !out && f-reach>0.001 { out=column_floor(c.x,c.y-c.z-0.001,c.y,s)-reach>0.001; }
        reached=reached || !out;
        let bevel=select(0.0,min(min(s.detail.z,s.arc.w*0.48),(1.0-s.geometry.w)*0.2),s.arc.w>0.0);
        // Any ink: a sample on the other side of `is_rim` takes the other band.
        let band=select(0.0,s.outline.w/(s.geometry.z*cs),
            max(max(s.outline.x,s.outline.y),s.outline.z)>0.0 && s.outline.w>0.0);
        // How far each sample's ray can move along the plane's normal.
        let rise=margin*sn/s.geometry.z;
        let on_top=f+bevel+reach+band< -0.001 && c.w-rise>0.05;
        if on_top {
            tops+=1u;top_t=(c.w+rise)/cs;
        } else if !out {
            // Above the top the ray is outside the solid; to reach it the
            // ray falls at least as far as its footprint is from zero.
            other=min(other,(c.w-rise)/cs+select(0.0,max(f-reach-0.001,0.0)/max(sn,0.000001),sn>0.0));
        }
        var amount=0.0;var inks=false;var ink=0.0;var a=0.0;var plain=0.0;
        if analytic {
            rays=rays && (out || c.w-rise>0.05);
            let e=edges(c.xy,s);
            let is_rim=min(abs(e.x),abs(e.y))<=abs(e.z);
            amount=max(select(s.outline.y,s.outline.x,is_rim),s.outline.z);
            inks=amount>0.0 && s.outline.w>0.0;
            // `outlined` at a hit on a top face: inked where one of the four
            // neighbours `band` px away along x or y misses the sector.
            let width=s.outline.w*select(0.5,1.0,is_rim);
            ink=d+width;
            if inks && d*cs+width>-margin && d*cs<margin {
                ink=-10000.0;
                let offsets=array<vec2<f32>,4>(vec2(width,0.0),vec2(-width,0.0),vec2(0.0,width),vec2(0.0,-width));
                for(var j=0u;j<4u;j++) {
                    ink=max(ink,footprint(top_plane(pixel+offsets[j],s).xy,s)*s.geometry.z);
                }
            }
            // In physical pixels at most: both move by at most 1 / cos(tilt)
            // times the distance on screen.
            let edge=min(abs(d),select(10000.0,abs(ink),inks))*cs;
            near=near || edge<margin;
            finite=finite && (edge>=margin || edge<margin);
            a=1.0-smoothstep(-0.5*aa,0.5*aa,d);
            plain=select(a,1.0-smoothstep(-0.5*aa,0.5*aa,ink),inks);
        }
        // One place shades a sector, at the first sample: every call is
        // inlined, and a second copy of the march and the material slowed
        // even Flat charts on SwiftShader.
        if a>0.0 || on_top {
            var h=hit_pixel(first,s);
            if h.t>=9999.0 { h=top_hit(first,s); }
            let surface=sector_surface(h,s);
            if on_top { shaded=surface; }
            let inked=select(surface,mix(surface,s.outline_color.rgb,s.outline_color.a*amount),inks);
            // Ink adds its difference from the fill over its band.
            covered+=surface*a+(inked-surface)*(a-plain);total+=a;
            if d<=0.0 && h.t<closest {
                closest=h.t;
                color=vec4(select(surface,inked,inks && ink>0.0),1.0);
            }
        }
    }
    if !finite { return FlatPixel(false,color); }
    if analytic {
        if !rays { return FlatPixel(false,color); }
        if near {
            // Overlapping coverage (more than one in all) shares the pixel.
            color=vec4(covered/max(total,1.0),min(total,1.0));
        } else if closest>=10000.0 { color=vec4(0.0); }
        return FlatPixel(true,color);
    }
    if !reached { return FlatPixel(true,vec4(0.0)); }
    if tops==1u && other>top_t+0.0001 { return FlatPixel(true,vec4(shaded,1.0)); }
    return FlatPixel(false,color);
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
    // `winner` counts sectors from 1, and 0 means none, as in the radial
    // pick: Metal compiles a 0-based `winner=i` as a constant 0 (#63).
    var closest=10000.0;var winner=0u;
    for(var i=0u;i<u32(globals.size.z);i++) {
        let h=hit_pixel(pixel,sectors[i]);
        if h.t<closest {closest=h.t;winner=i+1u;}
    }
    if winner!=0u {
        let s=sectors[winner-1u];
        if s.effects.z>0.0 && s.rounding.z>0.0 {
            let offset=vec2(0.0,s.effects.w);
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
    // The samples lie within 0.48 px of the center (a 3x3 grid's corner);
    // 0.75 px covers that times sqrt(2). The choice of path is uniform (it
    // reads read-only storage), so both keep their derivatives in uniform
    // control flow.
    var flat=FlatPixel(false,vec4(0.0));
    let level=flat_chart();
    if level { flat=flat_pixel(p.xy,0.75); } else { flat=material_pixel(p.xy,0.75); }
    // The floor shadow, a soft blur, is taken once at the pixel center: the
    // ground under the samples and under `material_pixel`'s colour.
    var ground=vec4(0.0);
    if !level || !flat.exact {
        let shade=floor_shadow(p.xy);
        ground=vec4(globals.background.rgb*globals.background.a*(1.0-shade),
            shade+globals.background.a*(1.0-shade));
    }
    // Branch once, outside the sample loops: the same choice made per sample
    // inside them left some pixels zero on lavapipe (Mesa 25.2.8).
    if flat.exact {
        // Nothing of the ground where a sector covers the pixel, all of it
        // where none does: the samples' own colour, bit for bit.
        if !level { flat.color+=ground*(1.0-flat.color.a); }
        // The same sum in the same order as the samples, so a pixel away
        // from every edge is bit-identical to theirs.
        for(var k=0u;k<grid*grid;k++) { color+=flat.color; }
    } else {
        // Supersample coverage, shade once per surface (#349). Every sample
        // finds the sector it hits, or the background, at its own position,
        // and takes its outline there, so edges and ink keep their
        // antialiasing. The lit material is evaluated once for the samples
        // that hit one sector with one normal, at the first of them, and
        // used for each, added in sample order: an interior pixel of a face
        // evaluates one material, while samples across a bevel or a crease
        // keep their own normal and highlight.
        // Indexed by sample: a grid side is at most 3 (`Globals::size.w`).
        var winners=array<u32,9>();var normals=array<vec3<f32>,9>();var surfaces=array<vec3<f32>,9>();
        for(var y=0u;y<grid;y++) { for(var x=0u;x<grid;x++) {
            let k=y*grid+x;
            let at=p.xy+(vec2(f32(x),f32(y))+0.5)/f32(grid)-0.5;
            let c=coverage(at);
            winners[k]=c.winner;normals[k]=c.hit.n;
            if c.winner==0u { color+=ground; continue; }
            // One normal: within about 2.5 degrees, so a face's curvature
            // across a pixel shares and a bevel's turn does not.
            var first=k;
            for(var j=0u;j<k;j++) {
                if winners[j]==c.winner && dot(normals[j],c.hit.n)>0.999 { first=j;break; }
            }
            let s=sectors[c.winner-1u];
            // Every slot is filled: the test is not transitive, so a later
            // sample may share with one that shared with an earlier one.
            if first==k { surfaces[k]=sector_surface(c.hit,s); } else { surfaces[k]=surfaces[first]; }
            color+=vec4(outlined(at,c.hit,s,surfaces[k]),1.0);
        }}
    }
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
