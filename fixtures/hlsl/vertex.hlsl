// The vertex shader of the HLSL demo: matrix transforms with both matrix layouts.
#include "common.hlsli"

VertexOutput main(VertexInput input)
{
    VertexOutput output;
    float4 world = mul(float4(input.position, 1.0f), g_world);
    output.position = mul(g_view_projection, world);
    float3 normals[2];
    normals[0] = mul(input.normal, g_normal_matrix);
    normals[1] = -normals[0];
    output.normal = normalize(normals[input.id & 1]);
    float2x2 turn = rotation((float)g_frame * 0.01f);
    output.uv = mul(input.uv, turn) + (float2)g_offsets[input.id & 1].xy / g_viewport;
    output.id = input.id;
    return output;
}
