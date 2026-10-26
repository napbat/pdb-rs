// The compute shader of the HLSL demo: structured and RW buffers, a byte address buffer,
// a RW texture and groupshared memory.
#include "common.hlsli"

#define GROUP_SIZE 64

StructuredBuffer<Particle> g_particles : register(t0);
RWStructuredBuffer<Particle> g_updated : register(u0);
RWByteAddressBuffer g_counters : register(u1);
RWTexture2D<float4> g_heat : register(u2);

groupshared float g_life[GROUP_SIZE];
groupshared uint g_alive;

[numthreads(GROUP_SIZE, 1, 1)]
void main(uint3 thread : SV_DispatchThreadID, uint index : SV_GroupIndex)
{
    if (index == 0)
    {
        g_alive = 0;
    }
    Particle particle = g_particles[thread.x];
    g_life[index] = particle.life;
    GroupMemoryBarrierWithGroupSync();

    float total = 0.0f;
    for (uint other = 0; other < GROUP_SIZE; other += 8)
    {
        total += g_life[other];
    }
    if (particle.life > 0.0f)
    {
        InterlockedAdd(g_alive, 1);
    }
    particle.position += float3(particle.velocity, 0.0f) * 0.016f;
    particle.life -= 0.016f;
    g_updated[thread.x] = particle;
    GroupMemoryBarrierWithGroupSync();

    if (index == 0)
    {
        g_counters.InterlockedAdd(0, g_alive);
    }
    min16float3 color = shade((min16float3)float3(total, particle.life, 0.0f), 0.5);
    g_heat[thread.xy] = float4(color, 1.0f);
}
