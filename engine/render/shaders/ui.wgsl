struct Screen {size:vec4<f32>};
@group(0) @binding(0) var<uniform> screen:Screen;
@group(0) @binding(1) var image:texture_2d<f32>;
@group(0) @binding(2) var image_sampler:sampler;
struct Input {@location(0) position:vec2<f32>,@location(1) uv:vec2<f32>,@location(2) color:vec4<f32>};
struct Output {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) color:vec4<f32>};
@vertex fn ui_vs(v:Input)->Output {var o:Output;o.position=vec4(v.position/screen.size.xy*vec2(2.,-2.)+vec2(-1.,1.),0.,1.);o.uv=v.uv;o.color=v.color;return o;}
@fragment fn ui_fs(v:Output)->@location(0) vec4<f32> {return v.color*textureSample(image,image_sampler,v.uv);}
