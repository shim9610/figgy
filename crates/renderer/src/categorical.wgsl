@fragment fn fs(v:Vertex)->@location(0) vec4<f32> {
    let p=v.position.xy;
    if v.layer==0u {
        let ink=textureLoad(annotations,vec2<i32>(p),0);
        let bg=vec4(globals.background.rgb*globals.background.a,globals.background.a);
        return output(ink+bg*(1.0-ink.a));
    }
    if v.layer>u32(globals.size.z) { return output(textureLoad(annotations,vec2<i32>(p)+vec2(0,i32(globals.size.y)),0)); }
    let b=bars[v.layer-1u];var coverage=0.0;var border=0.0;
    // Half-open ownership at shared stack edges; AA remains on the silhouette.
    // Two separately alpha-blended partial coverages must not expose background.
    let edge_mask=u32(b.effects.z);
    if ((edge_mask&1u)!=0u && p.x<b.rect.x) || ((edge_mask&2u)!=0u && p.x>=b.rect.z)
        || ((edge_mask&4u)!=0u && p.y<b.rect.y) || ((edge_mask&8u)!=0u && p.y>=b.rect.w) { discard; }
    for(var y=0u;y<2u;y++) { for(var x=0u;x<2u;x++) {
        var sample=p+(vec2(f32(x),f32(y))+0.5)*0.5-0.5;
        if (edge_mask&1u)!=0u { sample.x=max(sample.x,b.rect.x); }
        if (edge_mask&2u)!=0u { sample.x=min(sample.x,b.rect.z); }
        if (edge_mask&4u)!=0u { sample.y=max(sample.y,b.rect.y); }
        if (edge_mask&8u)!=0u { sample.y=min(sample.y,b.rect.w); }
        let d=sdf(sample,b);
        coverage+=select(0.0,0.25,d<=0.0);
        border+=select(0.0,0.25,d<=0.0 && d>=-b.outline.y && b.outline.x>0.5);
    }}
    let c=mix(shade(p,b),b.outline_color.rgb,b.outline_color.a*border/max(coverage,0.001));
    return output(vec4(c*coverage,coverage));
}
