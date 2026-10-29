// The radial pick: the sector under one physical pixel of a radial frame,
// the one nearest the camera, by the draw's own `hit_pixel` and occlusion
// rule (the first sector with the smallest ray distance wins, compared by
// `hit_key`), from the
// frame's own sector buffer. Spliced after `material.wgsl` and
// `data_render/common/radial_sector.wgsl` (SHADER_COMMON.md §5.12).
struct RadialPickQuery {
    pixel: vec2<f32>, // the physical pixel's centre
    count: u32, // sectors in the buffer
    rest: u32, // 1: every sector at rest, without its hover lift
};
struct RadialPickResult {
    index: u32, // the sector picked, when `hit` is 1
    hit: u32,
    t: f32, // its ray distance
    reserved: u32,
};
@group(0) @binding(0) var<uniform> query: RadialPickQuery;
@group(0) @binding(1) var<storage, read> sectors: array<Sector>;
@group(0) @binding(2) var<storage, read_write> result: RadialPickResult;
@compute @workgroup_size(1) fn pick() {
    var closest=10000.0;var key=hit_key(closest);var winner=0u;
    for(var i=0u;i<query.count;i++) {
        var s=sectors[i];
        if query.rest!=0u { s.rounding.z=0.0; }
        let h=hit_pixel(query.pixel,s);
        if hit_key(h.t)<key {closest=h.t;key=hit_key(h.t);winner=i;}
    }
    result=RadialPickResult(winner,select(0u,1u,closest<9999.0),closest,0u);
}
