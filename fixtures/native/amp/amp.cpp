// A C++ AMP program for MSVC 14.4x (MSVC 14.50 removed amp.h). The compiler writes the
// accelerator symbols: S_LPROC32_DPC (S_LPROC32_DPC_ID in the object file), the def-ranges
// S_DEFRANGE_DPC_PTR_TAG, S_DPC_SYM_TAG_MAP and S_LOCAL_DPC_GROUPSHARED for tile_static data.
// The program does not need to run: the fixture only uses its PDB.
#include <amp.h>
#include <cstdio>

using namespace concurrency;

static const int TILE = 4;

int main() {
    int data[16];
    for (int index = 0; index < 16; ++index) {
        data[index] = index;
    }
    array_view<int, 1> view(16, data);
    parallel_for_each(view.extent, [=](index<1> position) restrict(amp) {
        int doubled = view[position] * 2;
        view[position] = doubled + 1;
    });
    array_view<int, 1> tiled(16, data);
    parallel_for_each(tiled.extent.tile<TILE>(), [=](tiled_index<TILE> tile) restrict(amp) {
        tile_static int cache[TILE];
        int local = tile.local[0];
        cache[local] = tiled[tile.global];
        tile.barrier.wait();
        int sum = 0;
        for (int index = 0; index < TILE; ++index) {
            sum += cache[index];
        }
        tiled[tile.global] = sum;
    });
    view.synchronize();
    tiled.synchronize();
    std::printf("%d\n", data[3]);
    return 0;
}
