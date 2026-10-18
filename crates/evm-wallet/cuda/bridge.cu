#include <cuda_runtime.h>
#include <stdint.h>
#include <stdio.h>
#include <mutex>
#include "sha512_device.cuh"
#include "secp256k1_device.cuh"

// Large enough to occupy a Pascal GPU. The host checks cancellation between batches.
constexpr uint32_t kMaxBatch = 131072;
constexpr int kBlockSize = 256;

namespace {
__device__ __constant__ uint64_t kRound[24] = {
    0x0000000000000001ULL, 0x0000000000008082ULL, 0x800000000000808aULL,
    0x8000000080008000ULL, 0x000000000000808bULL, 0x0000000080000001ULL,
    0x8000000080008081ULL, 0x8000000000008009ULL, 0x000000000000008aULL,
    0x0000000000000088ULL, 0x0000000080008009ULL, 0x000000008000000aULL,
    0x000000008000808bULL, 0x800000000000008bULL, 0x8000000000008089ULL,
    0x8000000000008003ULL, 0x8000000000008002ULL, 0x8000000000000080ULL,
    0x000000000000800aULL, 0x800000008000000aULL, 0x8000000080008081ULL,
    0x8000000000008080ULL, 0x0000000080000001ULL, 0x8000000080008008ULL,
};
__device__ __constant__ int kRotation[25] = {
    0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15,
    21, 8, 18, 2, 61, 56, 14,
};
__device__ __constant__ char kHex[] = "0123456789abcdef";

__device__ __forceinline__ uint64_t rotate_left(uint64_t value, int amount) {
    return amount == 0 ? value : ((value << amount) | (value >> (64 - amount)));
}

__device__ void keccak_f(uint64_t state[25]) {
    uint64_t c[5], d[5], b[25];
    for (int round = 0; round < 24; ++round) {
        for (int x = 0; x < 5; ++x)
            c[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
        for (int x = 0; x < 5; ++x)
            d[x] = c[(x + 4) % 5] ^ rotate_left(c[(x + 1) % 5], 1);
        for (int y = 0; y < 5; ++y)
            for (int x = 0; x < 5; ++x)
                state[x + 5 * y] ^= d[x];
        for (int y = 0; y < 5; ++y)
            for (int x = 0; x < 5; ++x) {
                const int destination_x = y;
                const int destination_y = (2 * x + 3 * y) % 5;
                b[destination_x + 5 * destination_y] =
                    rotate_left(state[x + 5 * y], kRotation[x + 5 * y]);
            }
        for (int y = 0; y < 5; ++y)
            for (int x = 0; x < 5; ++x)
                state[x + 5 * y] = b[x + 5 * y] ^
                    ((~b[(x + 1) % 5 + 5 * y]) & b[(x + 2) % 5 + 5 * y]);
        state[0] ^= kRound[round];
    }
}

__device__ bool matches(const uint8_t digest[32], const char* target,
                        uint32_t target_len, uint32_t mode) {
    const uint32_t max_start = 40 - target_len;
    const uint32_t first = mode == 1 ? max_start : 0;
    const uint32_t last = mode == 0 ? 1 : max_start + 1;
    for (uint32_t start = first; start < last; ++start) {
        bool equal = true;
        for (uint32_t i = 0; i < target_len; ++i) {
            const uint8_t byte = digest[12 + ((start + i) >> 1)];
            const char c = ((start + i) & 1) ? kHex[byte & 15] : kHex[byte >> 4];
            if (c != target[i]) { equal = false; break; }
        }
        if (equal) return true;
    }
    return false;
}

__global__ void derive_and_hash(const uint8_t* parent_pubkey, const uint8_t* chain_code,
                                const char* target, uint32_t target_len, uint32_t mode,
                                uint32_t start_index, uint32_t count, uint32_t* result) {
    __shared__ uint8_t shared_parent[64];
    __shared__ uint8_t shared_chain[32];
    __shared__ char shared_target[40];
    if (threadIdx.x < 64) shared_parent[threadIdx.x] = parent_pubkey[threadIdx.x];
    if (threadIdx.x < 32) shared_chain[threadIdx.x] = chain_code[threadIdx.x];
    if (threadIdx.x < target_len && threadIdx.x < 40)
        shared_target[threadIdx.x] = target[threadIdx.x];
    __syncthreads();

    const uint32_t candidate = blockIdx.x * blockDim.x + threadIdx.x;
    if (candidate >= count) return;

    const uint32_t child_index = start_index + candidate;
    uint8_t compressed_parent[33];
    compressed_parent[0] = (shared_parent[63] & 1) ? 0x03 : 0x02;
    for (int i=0;i<32;++i) compressed_parent[i+1]=shared_parent[i];
    uint8_t digest512[64];
    evm_wallet_cuda::hmac_sha512_ckd(shared_chain, compressed_parent, child_index, digest512);
    if (!evm_wallet_cuda::scalar_valid(digest512)) return;
    const auto child = evm_wallet_cuda::ckdpub_point(digest512, shared_parent);
    evm_wallet_cuda::Fe child_x, child_y;
    if (!evm_wallet_cuda::point_to_affine(child, &child_x, &child_y)) return;
    uint8_t key[64];
    evm_wallet_cuda::fe_to_be(child_x, key);
    evm_wallet_cuda::fe_to_be(child_y, key + 32);

    uint64_t state[25] = {};
    for (uint32_t i = 0; i < 64; ++i)
        state[i >> 3] |= static_cast<uint64_t>(key[i]) << ((i & 7) * 8);
    state[8] |= 0x01ULL;
    state[16] |= 0x8000000000000000ULL;
    keccak_f(state);

    uint8_t digest[32];
    for (uint32_t i = 0; i < 32; ++i)
        digest[i] = static_cast<uint8_t>(state[i >> 3] >> ((i & 7) * 8));
    if (matches(digest, shared_target, target_len, mode))
        atomicMin(result, child_index);
}
}

struct DeviceBuffers {
    uint8_t* parent = nullptr;
    uint8_t* chain = nullptr;
    char* target = nullptr;
    uint32_t* result = nullptr;
    bool ready = false;
};

void release_buffers(DeviceBuffers& buffers) {
    if (buffers.result) cudaFree(buffers.result);
    if (buffers.target) cudaFree(buffers.target);
    if (buffers.chain) cudaFree(buffers.chain);
    if (buffers.parent) cudaFree(buffers.parent);
    buffers = {};
}

int ensure_buffers(DeviceBuffers& buffers) {
    if (buffers.ready) return 0;
    release_buffers(buffers);
    if (cudaSetDevice(0) != cudaSuccess) return -2;
    if (cudaMalloc(&buffers.parent, 64) != cudaSuccess ||
        cudaMalloc(&buffers.chain, 32) != cudaSuccess ||
        cudaMalloc(&buffers.target, 40) != cudaSuccess ||
        cudaMalloc(&buffers.result, sizeof(uint32_t)) != cudaSuccess) {
        release_buffers(buffers);
        return -2;
    }
    buffers.ready = true;
    return 0;
}

std::mutex& search_mutex() {
    static std::mutex mutex;
    return mutex;
}

DeviceBuffers& search_buffers() {
    static DeviceBuffers buffers;
    return buffers;
}

extern "C" int evm_wallet_cuda_available() {
    int count = 0;
    return cudaGetDeviceCount(&count) == cudaSuccess && count > 0 ? 1 : 0;
}

extern "C" uint32_t evm_wallet_cuda_max_batch() {
    return kMaxBatch;
}

extern "C" int evm_wallet_cuda_device_name(char* buffer, size_t buffer_len) {
    if (!buffer || buffer_len < 2) return -1;
    buffer[0] = '\0';
    cudaDeviceProp prop;
    if (cudaGetDeviceProperties(&prop, 0) != cudaSuccess) return -1;
    snprintf(buffer, buffer_len, "%s", prop.name);
    return buffer[0] == '\0' ? -1 : 0;
}

extern "C" int evm_wallet_cuda_search(const uint8_t* parent_pubkey,
                                       const uint8_t* chain_code,
                                       const char* target,
                                       size_t target_len,
                                       uint32_t mode,
                                       uint32_t start_index,
                                       uint32_t count,
                                       uint32_t* out_index) {
    if (!parent_pubkey || !chain_code || !target || !out_index || count == 0 || count > kMaxBatch ||
        target_len == 0 || target_len > 40 || mode > 2 ||
        start_index > 0x7fffffffU || count - 1 > 0x7fffffffU - start_index)
        return -1;

    std::lock_guard<std::mutex> lock(search_mutex());
    DeviceBuffers& buffers = search_buffers();
    if (ensure_buffers(buffers) != 0) return -2;
    if (cudaMemcpy(buffers.parent, parent_pubkey, 64, cudaMemcpyHostToDevice) != cudaSuccess ||
        cudaMemcpy(buffers.chain, chain_code, 32, cudaMemcpyHostToDevice) != cudaSuccess ||
        cudaMemcpy(buffers.target, target, target_len, cudaMemcpyHostToDevice) != cudaSuccess ||
        cudaMemset(buffers.result, 0xff, sizeof(uint32_t)) != cudaSuccess)
        return -2;

    derive_and_hash<<<(count + kBlockSize - 1) / kBlockSize, kBlockSize>>>(
        buffers.parent, buffers.chain, buffers.target,
        static_cast<uint32_t>(target_len), mode, start_index, count, buffers.result);
    if (cudaGetLastError() != cudaSuccess || cudaDeviceSynchronize() != cudaSuccess)
        return -3;

    uint32_t result = 0xffffffffU;
    if (cudaMemcpy(&result, buffers.result, sizeof(result), cudaMemcpyDeviceToHost) != cudaSuccess)
        return -2;
    if (result == 0xffffffffU) return 0;
    *out_index = result;
    return 1;
}

