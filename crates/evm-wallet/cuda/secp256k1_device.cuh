#pragma once
#include <stdint.h>

namespace evm_wallet_cuda {

struct Fe { uint64_t v[4]; };
struct Point { Fe x, y, z; };

__device__ __constant__ uint64_t kFieldP[4] = {
    0xfffffffefffffc2fULL, 0xffffffffffffffffULL,
    0xffffffffffffffffULL, 0xffffffffffffffffULL
};
__device__ __constant__ Fe kFieldR = {{0x00000001000003d1ULL, 0, 0, 0}};
__device__ __constant__ Fe kFieldR2 = {{0x000007a2000e90a1ULL, 1, 0, 0}};
__device__ __constant__ Fe kGeneratorX = {{
    0xd7362e5a487e2097ULL, 0x231e295329bc66dbULL,
    0x979f48c033fd129cULL, 0x9981e643e9089f48ULL}};
__device__ __constant__ Fe kGeneratorY = {{
    0xb15ea6d2d3dbabe2ULL, 0x8dfc5d5d1f1dc64dULL,
    0x70b6b59aac19c136ULL, 0xcf3f851fd4a582d6ULL}};
__device__ __constant__ uint64_t kScalarN[4] = {
    0xbfd25e8cd0364141ULL, 0xbaaedce6af48a03bULL,
    0xfffffffffffffffeULL, 0xffffffffffffffffULL
};

__device__ __forceinline__ bool fe_zero(const Fe& a) {
    return !(a.v[0] | a.v[1] | a.v[2] | a.v[3]);
}
__device__ __forceinline__ bool fe_equal(const Fe& a, const Fe& b) {
    return a.v[0] == b.v[0] && a.v[1] == b.v[1] &&
           a.v[2] == b.v[2] && a.v[3] == b.v[3];
}
__device__ __forceinline__ int fe_cmp(const Fe& a, const Fe& b) {
    for (int i = 3; i >= 0; --i) {
        if (a.v[i] < b.v[i]) return -1;
        if (a.v[i] > b.v[i]) return 1;
    }
    return 0;
}
__device__ __forceinline__ Fe fe_add(const Fe& a, const Fe& b) {
    Fe r; uint64_t carry = 0;
    #pragma unroll
    for (int i=0;i<4;++i) {
        const uint64_t x = a.v[i] + carry;
        const uint64_t c1 = x < a.v[i];
        r.v[i] = x + b.v[i];
        carry = c1 | (r.v[i] < x);
    }
    if (carry || fe_cmp(r, Fe{{kFieldP[0],kFieldP[1],kFieldP[2],kFieldP[3]}}) >= 0) {
        uint64_t borrow = 0;
        #pragma unroll
        for (int i=0;i<4;++i) {
            const uint64_t p = kFieldP[i];
            const uint64_t t = r.v[i] - p;
            const uint64_t b1 = r.v[i] < p;
            r.v[i] = t - borrow;
            borrow = b1 | (t < borrow);
        }
    }
    return r;
}
__device__ __forceinline__ Fe fe_sub(const Fe& a, const Fe& b) {
    Fe r; uint64_t borrow = 0;
    #pragma unroll
    for (int i=0;i<4;++i) {
        const uint64_t t = a.v[i] - b.v[i];
        const uint64_t b1 = a.v[i] < b.v[i];
        r.v[i] = t - borrow;
        borrow = b1 | (t < borrow);
    }
    if (borrow) {
        uint64_t carry = 0;
        #pragma unroll
        for (int i=0;i<4;++i) {
            const uint64_t t = r.v[i] + kFieldP[i];
            const uint64_t c1 = t < r.v[i];
            r.v[i] = t + carry;
            carry = c1 | (r.v[i] < t);
        }
    }
    return r;
}
__device__ __forceinline__ uint64_t mul_add_hi(uint64_t a, uint64_t b,
                                               uint64_t x, uint64_t y,
                                               uint64_t* low) {
    uint64_t lo = a * b;
    uint64_t hi = __umul64hi(a, b);
    uint64_t t = lo + x; hi += t < lo; lo = t;
    t = lo + y; hi += t < lo; *low = t;
    return hi;
}
__device__ __forceinline__ Fe fe_mont_mul(const Fe& a, const Fe& b) {
    uint64_t t[5] = {0,0,0,0,0};
    #pragma unroll
    for (int i=0;i<4;++i) {
        uint64_t carry = 0;
        #pragma unroll
        for (int j=0;j<4;++j) {
            uint64_t low;
            carry = mul_add_hi(a.v[j], b.v[i], t[j], carry, &low);
            t[j] = low;
        }
        uint64_t high_word = t[4] + carry;
        const uint64_t high_overflow = high_word < t[4];
        const uint64_t m = t[0] * 0xd838091dd2253531ULL;
        carry = 0;
        #pragma unroll
        for (int j=0;j<4;++j) {
            uint64_t low;
            carry = mul_add_hi(m, kFieldP[j], t[j], carry, &low);
            if (j > 0) t[j-1] = low;
        }
        uint64_t low;
        const uint64_t high = high_word + carry;
        const uint64_t overflow = high < high_word;
        low = high;
        t[3] = low;
        t[4] = overflow | high_overflow;
    }
    Fe r{{t[0],t[1],t[2],t[3]}};
    if (t[4] || fe_cmp(r, Fe{{kFieldP[0],kFieldP[1],kFieldP[2],kFieldP[3]}}) >= 0) {
        uint64_t borrow = 0;
        #pragma unroll
        for (int i=0;i<4;++i) {
            const uint64_t p = kFieldP[i];
            const uint64_t q = r.v[i] - p;
            const uint64_t b1 = r.v[i] < p;
            r.v[i] = q - borrow;
            borrow = b1 | (q < borrow);
        }
    }
    return r;
}
__device__ __forceinline__ Fe fe_mul(const Fe& a, const Fe& b) {
    return fe_mont_mul(a, b);
}
__device__ __forceinline__ Fe fe_from_mont(const Fe& a) {
    return fe_mont_mul(a, Fe{{1,0,0,0}});
}
__device__ __forceinline__ Fe fe_from_be(const uint8_t* bytes) {
    Fe r;
    #pragma unroll
    for (int limb=0;limb<4;++limb) {
        uint64_t x=0;
        #pragma unroll
        for(int j=0;j<8;++j) x=(x<<8)|bytes[32-(limb+1)*8+j];
        r.v[limb]=x;
    }
    return r;
}
__device__ __forceinline__ void fe_to_be(const Fe& mont, uint8_t* bytes) {
    const Fe r=fe_from_mont(mont);
    #pragma unroll
    for(int limb=0;limb<4;++limb)
        #pragma unroll
        for(int j=0;j<8;++j) bytes[32-(limb+1)*8+j]=uint8_t(r.v[limb]>>(56-8*j));
}
__device__ __forceinline__ Fe fe_parent_from_be(const uint8_t* bytes) {
    const Fe normal=fe_from_be(bytes);
    return fe_mont_mul(normal,kFieldR2);
}

__device__ __forceinline__ Point point_infinity() {
    return Point{Fe{{0,0,0,0}},Fe{{0,0,0,0}},Fe{{0,0,0,0}}};
}
__device__ __forceinline__ bool point_is_infinity(const Point& p) { return fe_zero(p.z); }
__device__ __forceinline__ Point point_double(const Point& p) {
    if (point_is_infinity(p) || fe_zero(p.y)) return point_infinity();
    const Fe a=fe_mul(p.x,p.x), b=fe_mul(p.y,p.y), c=fe_mul(b,b);
    const Fe xb=fe_add(p.x,b);
    Fe d=fe_sub(fe_sub(fe_mul(xb,xb),a),c); d=fe_add(d,d);
    Fe e=fe_add(fe_add(a,a),a); const Fe f=fe_mul(e,e);
    const Fe x=fe_sub(f,fe_add(d,d));
    Fe eightc=fe_add(c,c); eightc=fe_add(eightc,eightc); eightc=fe_add(eightc,eightc);
    const Fe y=fe_sub(fe_mul(e,fe_sub(d,x)),eightc);
    const Fe z=fe_add(fe_mul(p.y,p.z),fe_mul(p.y,p.z));
    return Point{x,y,z};
}
__device__ __forceinline__ Point point_add_mixed(const Point& p, const Fe& qx, const Fe& qy) {
    if (point_is_infinity(p)) return Point{qx,qy,kFieldR};
    const Fe z2=fe_mul(p.z,p.z), z3=fe_mul(z2,p.z);
    const Fe u=fe_mul(qx,z2), s=fe_mul(qy,z3);
    const Fe h=fe_sub(u,p.x), r=fe_sub(s,p.y);
    if (fe_zero(h)) return fe_zero(r) ? point_double(p) : point_infinity();
    const Fe hh=fe_mul(h,h), i=fe_add(fe_add(hh,hh),fe_add(hh,hh));
    const Fe j=fe_mul(h,i), rr=fe_add(r,r), v=fe_mul(p.x,i);
    const Fe x=fe_sub(fe_sub(fe_mul(rr,rr),j),fe_add(v,v));
    const Fe y=fe_sub(fe_mul(rr,fe_sub(v,x)),fe_add(fe_mul(p.y,j),fe_mul(p.y,j)));
    const Fe z=fe_sub(fe_sub(fe_mul(fe_add(p.z,h),fe_add(p.z,h)),z2),hh);
    return Point{x,y,z};
}
__device__ __forceinline__ bool scalar_valid(const uint8_t il[32]) {
    bool nonzero = false;
    for (int i = 0; i < 32; ++i) nonzero |= il[i] != 0;
    // BIP-32 rejects a zero tweak and a tweak that is not strictly below the curve order.
    if (!nonzero) return false;
    for (int i = 0; i < 32; ++i) {
        const uint8_t n = uint8_t(kScalarN[3 - (i / 8)] >> (56 - 8 * (i % 8)));
        if (il[i] < n) return true;
        if (il[i] > n) return false;
    }
    return false;
}
__device__ __forceinline__ Point ckdpub_point(const uint8_t il[32], const uint8_t parent[64]) {
    Point acc=point_infinity();
    for(int bit=0;bit<256;++bit) {
        if(!point_is_infinity(acc)) acc=point_double(acc);
        if((il[bit>>3]>>(7-(bit&7)))&1) acc=point_add_mixed(acc,kGeneratorX,kGeneratorY);
    }
    const Fe px=fe_parent_from_be(parent), py=fe_parent_from_be(parent+32);
    acc=point_add_mixed(acc,px,py);
    return acc;
}
__device__ __forceinline__ bool point_to_affine(Point p, Fe* x, Fe* y) {
    if(point_is_infinity(p)) return false;
    Fe zinv=kFieldR; // overwritten below by exponentiation p-2
    // Binary exponentiation with exponent p - 2 = ...fffffc2d.
    const uint8_t exp[32] = {
        0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,
        0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xfe,0xff,0xff,0xfc,0x2d};
    zinv=kFieldR;
    for(int i=0;i<256;++i) {
        zinv=fe_mul(zinv,zinv);
        if((exp[i>>3]>>(7-(i&7)))&1) zinv=fe_mul(zinv,p.z);
    }
    const Fe z2=fe_mul(zinv,zinv), z3=fe_mul(z2,zinv);
    *x=fe_mul(p.x,z2); *y=fe_mul(p.y,z3);
    return true;
}

} // namespace evm_wallet_cuda
