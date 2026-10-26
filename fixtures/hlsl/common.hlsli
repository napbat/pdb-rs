// Declarations that the shader stages share. The types and helpers give the debug
// information vectors, matrices with both layouts, small float types and inlined functions.
#ifndef PDBRS_COMMON_HLSLI
#define PDBRS_COMMON_HLSLI

cbuffer Frame : register(b0)
{
    row_major float4x4 g_world;
    column_major float4x4 g_view_projection;
    float3x3 g_normal_matrix;
    float4 g_tint;
    float2 g_viewport;
    uint g_frame;
    int3 g_offsets[2];
};

struct VertexInput
{
    float3 position : POSITION;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    uint id : SV_VertexID;
};

struct VertexOutput
{
    float4 position : SV_Position;
    float3 normal : NORMAL;
    float2 uv : TEXCOORD0;
    nointerpolation uint id : TEXCOORD1;
};

struct Particle
{
    float3 position;
    float life;
    min16float2 velocity;
    uint flags;
};

// A small helper with low-precision math. The compiler inlines it into each caller.
min16float3 shade(min16float3 color, min16float factor)
{
    min16float3 result = color * factor;
    return saturate(result + (min16float3)0.125);
}

float2x2 rotation(float angle)
{
    float s;
    float c;
    sincos(angle, s, c);
    return float2x2(c, -s, s, c);
}

#endif
