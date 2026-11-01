struct Globals {view_projection:mat4x4<f32>,light_projection:mat4x4<f32>,eye:vec4<f32>,light:vec4<f32>};
struct Object {model:mat4x4<f32>,normal:mat4x4<f32>,color:vec4<f32>,parameters:vec4<f32>};
@group(0) @binding(0) var<uniform> globals:Globals;
@group(0) @binding(1) var<uniform> object:Object;
@group(0) @binding(2) var base_image:texture_2d<f32>;
@group(0) @binding(3) var base_sampler:sampler;
@group(0) @binding(4) var shadow_image:texture_depth_2d;
@group(0) @binding(5) var shadow_sampler:sampler_comparison;
struct Input {@location(0) position:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>};
struct Output {@builtin(position) clip:vec4<f32>,@location(0) world:vec3<f32>,@location(1) normal:vec3<f32>,@location(2) uv:vec2<f32>,@location(3) light:vec4<f32>};
@vertex fn scene_vs(v:Input)->Output {
    let world=object.model*vec4(v.position,1.);var o:Output;o.clip=globals.view_projection*world;o.world=world.xyz;
    o.normal=(object.normal*vec4(v.normal,0.)).xyz;o.uv=v.uv;o.light=globals.light_projection*world;return o;
}
@vertex fn shadow_vs(v:Input)->@builtin(position) vec4<f32> {return globals.light_projection*object.model*vec4(v.position,1.);}
@fragment fn scene_fs(v:Output)->@location(0) vec4<f32> {
    let color=object.color.rgb*textureSample(base_image,base_sampler,v.uv).rgb;
    let n=normalize(v.normal);let l=normalize(globals.light.xyz);let view=normalize(globals.eye.xyz-v.world);let h=normalize(l+view);
    let nl=max(dot(n,l),0.);let nv=max(dot(n,view),0.001);let nh=max(dot(n,h),0.);let hv=max(dot(h,view),0.);
    let roughness=clamp(object.parameters.x,0.05,1.);let metallic=object.parameters.y;let a=roughness*roughness;let a2=a*a;
    let d=a2/(3.14159265*pow(nh*nh*(a2-1.)+1.,2.)+0.00001);
    let k=(roughness+1.)*(roughness+1.)/8.;let g=(nl/(nl*(1.-k)+k))*(nv/(nv*(1.-k)+k));
    let f0=mix(vec3(0.04),color,vec3(metallic));let f=f0+(vec3(1.)-f0)*pow(1.-hv,5.);
    let specular=d*g*f/max(4.*nl*nv,0.001);let diffuse=(vec3(1.)-f)*(1.-metallic)*color/3.14159265;
    let projected=v.light.xyz/v.light.w;let uv=vec2(projected.x*0.5+0.5,0.5-projected.y*0.5);
    let comparison=textureSampleCompare(shadow_image,shadow_sampler,uv,projected.z-0.002);
    let valid=all(uv>=vec2(0.)) && all(uv<=vec2(1.)) && projected.z>=0. && projected.z<=1.;
    let visibility=select(1.,comparison,valid && object.parameters.w>0.5);
    var lit=(diffuse+specular)*nl*3.5*visibility+color*0.12;
    lit+=object.parameters.z*vec3(0.06,0.035,0.005);lit=lit/(lit+vec3(1.));
    return vec4(pow(lit,vec3(1./2.2)),1.);
}
