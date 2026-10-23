fn sdf(p:vec2<f32>,b:Bar)->f32 {
    let q=p-(b.rect.xy+b.rect.zw)*0.5;
    var r=select(b.radii.w,b.radii.z,q.x>=0.0);
    if q.y<0.0 { r=select(b.radii.x,b.radii.y,q.x>=0.0); }
    let d=abs(q)-(b.rect.zw-b.rect.xy)*0.5+r;
    return length(max(d,vec2(0.0)))+min(max(d.x,d.y),0.0)-r;
}
fn shade(p:vec2<f32>,b:Bar)->vec3<f32> {
    let m=b.material;var c=b.color.rgb;let kind=u32(m.params.x);
    let uv=(p-b.rect.xy)/max(b.rect.zw-b.rect.xy,vec2(0.001));
    let across=select(uv.x,uv.y,b.outline.z>0.5);
    let logical=(p-b.rect.xy)/b.effects.y;
    let grain=noise(logical*m.params.w*0.6+vec2(m.finish.y));
    if kind==MATERIAL_MATTE { c*=0.98+(grain-0.5)*m.params.z*0.08+0.04*(1.0-uv.y); }
    if kind==MATERIAL_SATIN_METAL {
        let brush=noise(vec2(logical.x*0.06,logical.y*1.5)*m.params.w);
        let light=exp(-pow((across-0.25)/0.23,2.0));
        c=c*(0.82+(brush-0.5)*m.params.z*0.09)+vec3(light*m.finish.x*0.25);
    }
    if kind==MATERIAL_ENAMEL { c=c*(0.92+0.08*(1.0-across))+vec3(exp(-pow((across-0.18)/0.16,2.0))*m.finish.x*0.16); }
    if kind==MATERIAL_PAPER { c*=1.0+(grain-0.5)*m.params.z*0.16; }
    return clamp(c+vec3(m.finish.w),vec3(0.0),vec3(1.0));
}
fn output(c:vec4<f32>)->vec4<f32> {
    if globals.size.w>0.5 && c.a>0.0 {
        let rgb=c.rgb/c.a;let linear=select(pow((rgb+0.055)/1.055,vec3(2.4)),rgb/12.92,rgb<=vec3(0.04045));
        return vec4(linear*c.a,c.a);
    }
    return c;
}
