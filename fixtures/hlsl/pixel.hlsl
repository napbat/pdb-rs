// The pixel shader of the HLSL demo: textures, samplers, a texture array and a structured
// buffer.
#include "common.hlsli"

Texture2D<float4> g_albedo : register(t0);
Texture2DArray<float> g_shadows : register(t1);
StructuredBuffer<float4> g_palette : register(t2);
SamplerState g_linear : register(s0);
SamplerComparisonState g_compare : register(s1);

float4 main(VertexOutput input) : SV_Target
{
    float4 albedo = g_albedo.Sample(g_linear, input.uv);
    float shadow = g_shadows.SampleCmpLevelZero(g_compare, float3(input.uv, 0.0f), 0.5f);
    min16float3 lit = shade((min16float3)albedo.rgb, (min16float)shadow);
    float4 palette = g_palette[input.id % 4];
    [unroll]
    for (int index = 0; index < 2; ++index)
    {
        lit += (min16float3)(palette.rgb * 0.5f);
    }
    half brightness = (half)0.75;
    return float4(lit * brightness, albedo.a) * g_tint;
}
