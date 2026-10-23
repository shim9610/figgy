// The bounded charts' colour output: a premultiplied colour in sRGB, linearized
// when the target is an sRGB format (`globals.size.w` set), so the hardware's
// encode returns the same bytes. Each module declares its own `Globals`
// uniform `globals` with `size` as a member.
fn output(c:vec4<f32>)->vec4<f32> {
    if globals.size.w>0.5 && c.a>0.0 {
        let rgb=c.rgb/c.a;let linear=select(pow((rgb+0.055)/1.055,vec3(2.4)),rgb/12.92,rgb<=vec3(0.04045));
        return vec4(linear*c.a,c.a);
    }
    return c;
}
