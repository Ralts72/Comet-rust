#include <metal_stdlib>
#include <metal_raytracing>
using namespace metal;
using namespace raytracing;
kernel void workload(device float* result [[buffer(0)]], uint id [[thread_position_in_grid]]) {
    float x = float(id % 127) * 0.001f;
    for (uint i=0;i<512;i++) x = sin(x) + 0.01f;
    result[id] = x;
}
kernel void trace_triangle(primitive_acceleration_structure scene [[buffer(0)]],
                           device float* distances [[buffer(1)]], uint id [[thread_position_in_grid]]) {
    ray r;
    r.origin = float3(id == 0 ? 0.0f : 4.0f, 0.0f, 2.0f);
    r.direction = float3(0,0,-1); r.min_distance=0; r.max_distance=100;
    intersector<triangle_data> query;
    query.assume_geometry_type(geometry_type::triangle);
    auto hit = query.intersect(r, scene);
    distances[id] = hit.type == intersection_type::none ? -1.0f : hit.distance;
}
