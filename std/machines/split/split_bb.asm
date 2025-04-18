use std::prelude::Query;
use super::ByteCompare;

// Splits an arbitrary field element into two u16s, on the BabyBear field.
machine SplitBB(byte_compare: ByteCompare) with
    latch: RESET,
    // Allow this machine to be connected via a permutation
    call_selectors: sel,
{
    operation split in_acc -> output_low, output_high;

    // Latch and operation ID
    col fixed RESET(i) { if i % 4 == 3 { 1 } else { 0 } };

    // 1. Decompose the input into bytes

    // The byte decomposition of the input, in little-endian order
    // and shifted forward by one (to use the last row of the
    // previous block)
    // A hint is provided because automatic witness generation does not
    // understand step 3 to figure out that the byte decomposition is unique.
    let select_byte: fe, int -> fe = |input, byte| std::convert::fe((std::convert::int(input) >> (byte * 8)) & 0xff);
    col witness bytes;
    query |i| std::prover::compute_from(
        bytes,
        i,
        [in_acc'],
        |in_acc_next| select_byte(in_acc_next[0], (i + 1) % 4)
    );
    // Puts the bytes together to form the input
    col witness in_acc;
    // Factors to multiply the bytes by
    col fixed FACTOR(i) { 1 << (((i + 1) % 4) * 8) };

    in_acc' = (1 - RESET) * in_acc + bytes * FACTOR;

    // 2. Build the output, packing chunks of 2 bytes (i.e., 16 bit) into a field element
    col witness output_low, output_high;
    col fixed FACTOR_OUTPUT_LOW = [0x100, 0, 0, 1]*;
    col fixed FACTOR_OUTPUT_HIGH = [0, 1, 0x100, 0]*;
    output_low' = (1 - RESET) * output_low + bytes * FACTOR_OUTPUT_LOW;
    output_high' = (1 - RESET) * output_high + bytes * FACTOR_OUTPUT_HIGH;

    // 3. Check that the byte decomposition does not overflow
    //
    //    Skipping this step would work but it wouldn't be sound, because
    //    the 4-byte decomposition could overflow, since the BabyBear
    //    prime 2**31 - 2**27 + 1 is smaller than 2^32.
    //
    //    The approach is to compare the byte decomposition with that of
    //    the maximum possible value (0x78000000) byte by byte,
    //    from most significant to least significant (i.e., going backwards).
    //    A byte can only be larger than that of the max value if any previous
    //    byte has been smaller.

    // This is an example for input 0x77ffffff:
    // Row     RESET   bytes   BYTES_MAX  lt      was_lt  gt
    // -1      0x1     0xff    0x0        0x0     0x1     0x1
    //  0      0x0     0xff    0x0        0x0     0x1     0x1
    //  1      0x0     0xff    0x0        0x0     0x1     0x1
    //  2      0x0     0x77    0x78       0x1     0x1     0x0  # 0x77 < 0x78, so now greater bytes are allowed

    // Bytes of the maximum value, in little endian order, rotated by one
    col fixed BYTES_MAX = [0, 0, 0x78, 0]*;

    // Compare the current byte with the corresponding byte of the maximum value.
    col witness lt;
    col witness gt;
    link => (lt, gt) = byte_compare.run(bytes, BYTES_MAX);

    // Compute whether the current or any previous byte has been less than
    // the corresponding byte of the maximum value.
    // This moves *backward* from the second to last row.
    col witness was_lt;
    was_lt = RESET' * lt + (1 - RESET') * (was_lt' + lt - was_lt' * lt);

    // If any byte is larger, but no previous byte was smaller, the byte
    // decomposition has overflowed and should be rejected.
    gt * (1 - was_lt) = 0;
}
