// Verfies that the input is a palindrome.
// Input: length, x_1, x_2, ..., x_length

degree 1024;

reg pc[@pc];
reg X[<=];
reg A;
reg B;
reg I;
reg CNT;
reg ADDR;

pil{
    col witness XInv;
    col witness XIsZero;
    XIsZero  = 1 - X * XInv;
    XIsZero * X = 0;
    XIsZero * (1 - XIsZero) = 0;

    /// Write-once memory (actually a key-value store)
    col witness m_addr;
    col witness m_value;

    // positive numbers (assumed to be less than half the field order)
    col fixed POSITIVE(i) { i + 1 };
    col fixed FIRST = [1] + [0]*;
    col fixed NOTLAST(i) { 1 - FIRST(i + 1) };

    // This enforces that addresses are stored in an ascending order
    // (and in particular, are unique).
    NOTLAST { m_addr' - m_addr } in POSITIVE;
}

instr jmpz X, l: label { pc' = XIsZero * l + (1 - XIsZero) * (pc + 1) }
instr jmp l: label { pc' = l }
instr assert_zero X { XIsZero = 1 }
instr mstore X { { ADDR, X } in { m_addr, m_value } }
// TODO instructions that return values are currently rather clumsy.
// We should replace them by some function notion instead.
instr mload -> X { { ADDR, X } in { m_addr, m_value } }

CNT <=X= ${ ("input", 0) };
ADDR <=X= 0;
mstore CNT;

store_values::
 jmpz CNT, check_start;
 ADDR <=X= CNT;
 mstore ${ ("input", CNT) };
 CNT <=X= CNT - 1;
 jmp store_values;

check_start::
 ADDR <=X= 0;
 mload CNT;
 I <=X= 0;

check::
 jmpz I - CNT, end;
 ADDR <=X= I + 1;
 mload A;
 ADDR <=X= CNT - I;
 mload B;
 assert_zero A - B;
 I <=X= I + 1;
 jmp check;

end::
 jmp end;
