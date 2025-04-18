//! The concrete parameters used in the prover
//!
//! Inspired from [this example](https://github.com/Plonky3/Plonky3/blob/51c98987d1ee52c83a75142c1a2827d3ec71e563/keccak-air/examples/prove_koala_bear_poseidon2.rs)

use lazy_static::lazy_static;

use crate::params::{Challenger, FieldElementMap, Plonky3Field};
use p3_challenger::DuplexChallenger;
use p3_commit::ExtensionMmcs;
use p3_dft::Radix2DitParallel;
use p3_field::{extension::BinomialExtensionField, Field};
use p3_fri::{FriConfig, TwoAdicFriPcs};
use p3_koala_bear::{DiffusionMatrixKoalaBear, KoalaBear};
use p3_merkle_tree::MerkleTreeMmcs;
use p3_poseidon2::{poseidon2_round_numbers_128, Poseidon2, Poseidon2ExternalMatrixGeneral};
use p3_symmetric::{PaddingFreeSponge, TruncatedPermutation};
use p3_uni_stark::StarkConfig;

use crate::params::poseidon2;

use powdr_number::KoalaBearField;

const D: u64 = 3;
const WIDTH: usize = 16;
type Perm =
    Poseidon2<KoalaBear, Poseidon2ExternalMatrixGeneral, DiffusionMatrixKoalaBear, WIDTH, D>;

const DEGREE: usize = 4;
type FriChallenge = BinomialExtensionField<KoalaBear, DEGREE>;

const RATE: usize = 8;
const OUT: usize = 8;
type FriChallenger = DuplexChallenger<KoalaBear, Perm, WIDTH, RATE>;
type Hash = PaddingFreeSponge<Perm, WIDTH, RATE, OUT>;

const N: usize = 2;
const CHUNK: usize = 8;
type Compress = TruncatedPermutation<Perm, N, CHUNK, WIDTH>;
const DIGEST_ELEMS: usize = 8;
type ValMmcs = MerkleTreeMmcs<
    <KoalaBear as Field>::Packing,
    <KoalaBear as Field>::Packing,
    Hash,
    Compress,
    DIGEST_ELEMS,
>;

type ChallengeMmcs = ExtensionMmcs<KoalaBear, FriChallenge, ValMmcs>;
type Dft = Radix2DitParallel<KoalaBear>;
type MyPcs = TwoAdicFriPcs<KoalaBear, Dft, ValMmcs, ChallengeMmcs>;

const FRI_LOG_BLOWUP: usize = 1;
const FRI_NUM_QUERIES: usize = 100;
const FRI_PROOF_OF_WORK_BITS: usize = 16;

lazy_static! {
    static ref ROUNDS: (usize, usize) = poseidon2_round_numbers_128::<KoalaBear>(WIDTH, D);
    static ref ROUNDS_F: usize = ROUNDS.0;
    static ref ROUNDS_P: usize = ROUNDS.1;
    static ref PERM_BB: Perm = Perm::new(
        *ROUNDS_F,
        poseidon2::external_constants(*ROUNDS_F),
        Poseidon2ExternalMatrixGeneral,
        *ROUNDS_P,
        poseidon2::internal_constants(*ROUNDS_P),
        DiffusionMatrixKoalaBear::default()
    );
}

impl FieldElementMap for KoalaBearField {
    type Config = StarkConfig<MyPcs, FriChallenge, FriChallenger>;
    fn into_p3_field(self) -> Plonky3Field<Self> {
        self.into_inner()
    }

    fn from_p3_field(e: Plonky3Field<Self>) -> Self {
        KoalaBearField::from_inner(e)
    }

    fn get_challenger() -> Challenger<Self> {
        FriChallenger::new(PERM_BB.clone())
    }

    fn get_config() -> Self::Config {
        let hash = Hash::new(PERM_BB.clone());

        let compress = Compress::new(PERM_BB.clone());

        let val_mmcs = ValMmcs::new(hash, compress);

        let challenge_mmcs = ChallengeMmcs::new(val_mmcs.clone());

        let dft = Dft::default();

        let fri_config = FriConfig {
            log_blowup: FRI_LOG_BLOWUP,
            num_queries: FRI_NUM_QUERIES,
            proof_of_work_bits: FRI_PROOF_OF_WORK_BITS,
            mmcs: challenge_mmcs,
        };

        let pcs = MyPcs::new(dft, val_mmcs, fri_config);

        Self::Config::new(pcs)
    }

    fn degree_bound() -> usize {
        // Currently, Plonky3 can't compute evaluations other than those already computed for the
        // FRI commitment. This introduces the following dependency between the blowup factor and
        // the degree bound:
        (1 << FRI_LOG_BLOWUP) + 1
    }
}
