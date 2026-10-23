@fragment fn fs(v:Vertex)->@location(0) vec4<f32> {
    let p=v.position.xy;
    if v.layer==0u {
        let ink=textureLoad(annotations,vec2<i32>(p),0);
        let bg=vec4(globals.background.rgb*globals.background.a,globals.background.a);
        return output(ink+bg*(1.0-ink.a));
    }
    if v.layer>u32(globals.size.z) { return output(textureLoad(annotations,vec2<i32>(p)+vec2(0,i32(globals.size.y)),0)); }
    if p.x<globals.clip.x || p.y<globals.clip.y || p.x>=globals.clip.z || p.y>=globals.clip.w { discard; }
    let b=bars[v.layer-1u];var color=vec4(0.0);
    let fill=shade(p,b);
    for(var y=0u;y<2u;y++) { for(var x=0u;x<2u;x++) {
        let sample=p+(vec2(f32(x),f32(y))+0.5)*0.5-0.5;
        let d=box_sdf(sample,b);
        if d<=0.0 {
            var alpha=b.color.a;var rgb=fill*alpha;
            if b.outline.x>0.5 && d>=-b.outline.y {
                rgb=b.outline_color.rgb*b.outline_color.a+rgb*(1.0-b.outline_color.a);
                alpha=b.outline_color.a+alpha*(1.0-b.outline_color.a);
            }
            color+=vec4(rgb,alpha)*0.25;
        }
    }}
    return output(color);
}
