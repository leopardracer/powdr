#![no_main]
#![no_std]

use powdr_riscv_runtime::{
    goldilocks::{extract_opaque_vec, Goldilocks, OpaqueGoldilocks, PRIME},
    hash::{poseidon2_gl_compression, poseidon2_gl_inplace, Poseidon2OutputHalf},
};

#[no_mangle]
fn main() {
    let i = [OpaqueGoldilocks::from(0); 8];
    let h = extract_opaque_vec::<4>(&poseidon2_gl_compression(
        &i,
        Poseidon2OutputHalf::FirstHalf,
    ));
    assert_eq!(h[0], 14905565590733827480);
    assert_eq!(h[1], 640905753703258831);
    assert_eq!(h[2], 4579128623722792381);
    assert_eq!(h[3], 158153743058056413);

    let i = [OpaqueGoldilocks::from(1); 8];
    let h = extract_opaque_vec::<4>(&poseidon2_gl_compression(
        &i,
        Poseidon2OutputHalf::SecondHalf,
    ));
    assert_eq!(h[0], 2971437633000883992);
    assert_eq!(h[1], 9752341516515962403);
    assert_eq!(h[2], 15477293561177957600);
    assert_eq!(h[3], 13574628582471329853);

    let minus_one = PRIME - 1;
    let i = [OpaqueGoldilocks::from(Goldilocks::new(minus_one)); 8];
    let h = extract_opaque_vec::<4>(&poseidon2_gl_compression(
        &i,
        Poseidon2OutputHalf::FirstHalf,
    ));
    assert_eq!(h[0], 13601391594672984423);
    assert_eq!(h[1], 7799837486760213030);
    assert_eq!(h[2], 4721195013230721931);
    assert_eq!(h[3], 6190752424007146655);

    // Also test the inplace version
    let mut i = [
        923978,
        235763497586,
        9827635653498,
        112870,
        289273673480943876,
        230295874986745876,
        6254867324987,
        2087,
    ]
    .map(|x| OpaqueGoldilocks::from(Goldilocks::new(x)));
    poseidon2_gl_inplace(&mut i);

    let h = extract_opaque_vec::<8>(&i);
    assert_eq!(h[0], 14498150941209346562);
    assert_eq!(h[1], 8038616707062714447);
    assert_eq!(h[2], 17242548914990530484);
    assert_eq!(h[3], 3240738938335106853);
    assert_eq!(h[4], 13554879377661635843);
    assert_eq!(h[5], 12505236434419724338);
    assert_eq!(h[6], 3134668969942435695);
    assert_eq!(h[7], 1912726109528180442);
}
