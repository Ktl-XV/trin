use alloy::{consensus::Header, primitives::B256};
use jsonrpsee::core::Serialize;
use serde::Deserialize;
use ssz::SszDecoderBuilder;
use ssz_derive::{Decode, Encode};
use ssz_types::{typenum, FixedVector, VariableList};
use tree_hash::TreeHash;

use crate::types::{
    bytes::ByteList1024,
    consensus::{
        beacon_block::{BeaconBlockBellatrix, BeaconBlockCapella},
        beacon_state::{BeaconStateCapella, HistoricalBatch},
        proof::build_merkle_proof_for_index,
    },
    execution::{
        block_body::{MERGE_TIMESTAMP, SHANGHAI_TIMESTAMP},
        ssz_header,
    },
};

/// The accumulator proof for EL BlockHeader for the pre-merge blocks.
pub type BlockProofHistoricalHashesAccumulator = FixedVector<B256, typenum::U15>;

/// Proof that EL block_hash is in BeaconBlock -> BeaconBlockBody -> ExecutionPayload
/// for TheMerge until Capella
pub type ExecutionBlockProof = FixedVector<B256, typenum::U11>;
/// Proof that EL block_hash is in BeaconBlock -> BeaconBlockBody -> ExecutionPayload
/// for Post-Capella
pub type ExecutionBlockProofCapella = VariableList<B256, typenum::U12>;
/// Proof that BeaconBlock root is part of historical_summaries and thus canonical
/// for Capella and onwards
pub type BeaconBlockProofHistoricalSummaries = FixedVector<B256, typenum::U13>;
/// Proof that BeaconBlock root is part of historical_roots and thus canonical
/// from TheMerge until Capella -> Bellatrix fork.
pub type BeaconBlockProofHistoricalRoots = FixedVector<B256, typenum::U14>;

/// A block header with accumulator proof.
/// Type definition:
/// https://github.com/status-im/nimbus-eth1/blob/master/fluffy/network/history/history_content.nim#L136
#[derive(Debug, Clone, PartialEq, Eq, Encode, Deserialize)]
pub struct HeaderWithProof {
    #[ssz(with = "ssz_header")]
    pub header: Header,
    pub proof: BlockHeaderProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum BlockHeaderProof {
    // Pre-Merge
    HistoricalHashes(BlockProofHistoricalHashesAccumulator),
    // Merge -> Capella
    HistoricalRoots(BlockProofHistoricalRoots),
    // Post-Capella
    HistoricalSummaries(BlockProofHistoricalSummaries),
}

impl ssz::Decode for HeaderWithProof {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let mut builder = SszDecoderBuilder::new(bytes);

        builder.register_anonymous_variable_length_item()?;
        builder.register_anonymous_variable_length_item()?;

        let mut decoder = builder.build()?;

        let header = decoder.decode_next_with(ssz_header::decode::from_ssz_bytes)?;
        let proof = decoder.decode_next::<ByteList1024>()?;
        let proof = if header.timestamp <= MERGE_TIMESTAMP {
            BlockHeaderProof::HistoricalHashes(
                BlockProofHistoricalHashesAccumulator::from_ssz_bytes(&proof)?,
            )
        } else if header.timestamp <= SHANGHAI_TIMESTAMP {
            BlockHeaderProof::HistoricalRoots(BlockProofHistoricalRoots::from_ssz_bytes(&proof)?)
        } else {
            BlockHeaderProof::HistoricalSummaries(BlockProofHistoricalSummaries::from_ssz_bytes(
                &proof,
            )?)
        };
        Ok(Self { header, proof })
    }
}

impl ssz::Encode for BlockHeaderProof {
    fn is_ssz_fixed_len() -> bool {
        false
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        match self {
            BlockHeaderProof::HistoricalHashes(proof) => {
                proof.ssz_append(buf);
            }
            BlockHeaderProof::HistoricalRoots(proof) => {
                proof.ssz_append(buf);
            }
            BlockHeaderProof::HistoricalSummaries(proof) => {
                proof.ssz_append(buf);
            }
        }
    }

    fn ssz_bytes_len(&self) -> usize {
        match self {
            BlockHeaderProof::HistoricalHashes(proof) => proof.ssz_bytes_len(),
            BlockHeaderProof::HistoricalRoots(proof) => proof.ssz_bytes_len(),
            BlockHeaderProof::HistoricalSummaries(proof) => proof.ssz_bytes_len(),
        }
    }
}

/// The struct holds a chain of proofs. This chain of proofs allows for verifying that an EL
/// `BlockHeader` is part of the canonical chain. The only requirement is having access to the
/// beacon chain `historical_roots`.
///
/// Proof for EL BlockHeader from TheMerge until Capella
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
pub struct BlockProofHistoricalRoots {
    /// Proof that the BeaconBlock is part of the historical roots
    /// and thus part of the canonical chain.
    pub beacon_block_proof: BeaconBlockProofHistoricalRoots,
    /// hash_tree_root of BeaconBlock used to verify the proofs
    pub beacon_block_root: B256,
    /// Proof that EL BlockHash is part of the BeaconBlock
    pub execution_block_proof: ExecutionBlockProof,
    /// Slot of BeaconBlock, used to calculate the historical_roots index
    pub slot: u64,
}

/// The struct holds a chain of proofs. This chain of proofs allows for verifying that an EL
/// `BlockHeader` is part of the canonical chain. The only requirement is having access to the
/// beacon chain `historical_summaries`.
///
/// Proof for EL BlockHeader for Capella and onwards
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
pub struct BlockProofHistoricalSummaries {
    /// Proof that the BeaconBlock is part of the historical_summaries
    /// and thus part of the canonical chain.
    pub beacon_block_proof: BeaconBlockProofHistoricalSummaries,
    /// hash_tree_root of BeaconBlock used to verify the proofs
    pub beacon_block_root: B256,
    /// Proof that EL BlockHash is part of the BeaconBlock
    pub execution_block_proof: ExecutionBlockProofCapella,
    /// Slot of BeaconBlock, used to calculate the historical_summaries index
    pub slot: u64,
}

pub fn build_historical_roots_proof(
    slot: u64,
    historical_batch: &HistoricalBatch,
    beacon_block: BeaconBlockBellatrix,
) -> BlockProofHistoricalRoots {
    let beacon_block_proof = historical_batch.build_block_root_proof(slot % 8192);

    // execution block proof
    let mut execution_block_hash_proof = beacon_block.body.build_execution_block_hash_proof();
    let body_root_proof = beacon_block.build_body_root_proof();
    execution_block_hash_proof.extend(body_root_proof);

    BlockProofHistoricalRoots {
        beacon_block_proof: beacon_block_proof.into(),
        beacon_block_root: beacon_block.tree_hash_root(),
        execution_block_proof: execution_block_hash_proof.into(),
        slot,
    }
}

pub fn build_historical_summaries_proof(
    slot: u64,
    capella_state: &BeaconStateCapella,
    beacon_block: BeaconBlockCapella,
) -> BlockProofHistoricalSummaries {
    // beacon block proof
    let block_root_proof = capella_state.build_block_root_proof(slot as usize % 8192);
    let beacon_block_proof: FixedVector<B256, typenum::U13> = block_root_proof.into();

    // execution block proof
    let mut execution_block_hash_proof = beacon_block.body.build_execution_block_hash_proof();
    let body_root_proof = beacon_block.build_body_root_proof();
    execution_block_hash_proof.extend(body_root_proof);

    BlockProofHistoricalSummaries {
        beacon_block_proof,
        beacon_block_root: beacon_block.tree_hash_root(),
        execution_block_proof: execution_block_hash_proof.into(),
        slot,
    }
}

pub fn build_block_proof_historical_roots(
    slot: u64,
    historical_batch: HistoricalBatch,
    beacon_block: BeaconBlockBellatrix,
) -> BlockProofHistoricalRoots {
    // beacon block proof
    let historical_batch_proof = historical_batch.build_block_root_proof(slot % 8192);

    // execution block proof
    let mut execution_block_hash_proof = beacon_block.body.build_execution_block_hash_proof();
    let body_root_proof = beacon_block.build_body_root_proof();
    execution_block_hash_proof.extend(body_root_proof);

    BlockProofHistoricalRoots {
        beacon_block_proof: historical_batch_proof.into(),
        beacon_block_root: beacon_block.tree_hash_root(),
        execution_block_proof: execution_block_hash_proof.into(),
        slot,
    }
}

pub fn build_block_proof_historical_summaries(
    slot: u64,
    // block roots fields from BeaconState
    block_roots: FixedVector<B256, typenum::U8192>,
    beacon_block: BeaconBlockCapella,
) -> BlockProofHistoricalSummaries {
    // beacon block proof
    let leaves = block_roots
        .iter()
        .map(|root| root.tree_hash_root().0)
        .collect();
    let slot_index = slot as usize % 8192;
    let block_root_proof = build_merkle_proof_for_index(leaves, slot_index);
    let beacon_block_proof: FixedVector<B256, typenum::U13> = block_root_proof.into();

    // execution block proof
    let mut execution_block_hash_proof = beacon_block.body.build_execution_block_hash_proof();
    let body_root_proof = beacon_block.build_body_root_proof();
    execution_block_hash_proof.extend(body_root_proof);

    BlockProofHistoricalSummaries {
        beacon_block_proof,
        beacon_block_root: beacon_block.tree_hash_root(),
        execution_block_proof: execution_block_hash_proof.into(),
        slot,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::Value;
    use serde_yaml::Value as YamlValue;
    use ssz::Decode;

    use super::*;
    use crate::{
        test_utils::{read_bytes_from_tests_submodule, read_file_from_tests_submodule},
        types::consensus::{beacon_state::BeaconState, fork::ForkName},
        utils::bytes::{hex_decode, hex_encode},
    };

    #[test_log::test]
    fn decode_encode_headers_with_proof() {
        let file = read_file_from_tests_submodule(
            "tests/mainnet/history/headers_with_proof/1000001-1000010.json",
        )
        .unwrap();
        let json: Value = serde_json::from_str(&file).unwrap();
        let hwps = json.as_object().unwrap();
        for (block_number, obj) in hwps {
            let block_number: u64 = block_number.parse().unwrap();
            let actual_hwp = obj.get("content_value").unwrap().as_str().unwrap();
            let hwp = HeaderWithProof::from_ssz_bytes(&hex_decode(actual_hwp).unwrap()).unwrap();
            assert_eq!(block_number, hwp.header.number);
            let encoded = hex_encode(ssz::Encode::as_ssz_bytes(&hwp));
            assert_eq!(encoded, actual_hwp);
        }
    }

    #[rstest::rstest]
    #[case("1000010")]
    #[case("14764013")]
    #[case("15537392")]
    #[case("15537393")]
    #[case("15539558")]
    #[case("15547621")]
    #[case("15555729")]
    #[case("17034870")]
    #[case("17042287")]
    #[case("17062257")]
    fn decode_encode_more_headers_with_proofs(#[case] filename: &str) {
        let file = read_file_from_tests_submodule(format!(
            "tests/mainnet/history/headers_with_proof/{filename}.yaml"
        ))
        .unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&file).unwrap();
        let actual_hwp = yaml.get("content_value").unwrap().as_str().unwrap();
        let hwp = HeaderWithProof::from_ssz_bytes(&hex_decode(actual_hwp).unwrap()).unwrap();
        assert_eq!(hwp.header.number, filename.parse::<u64>().unwrap());
        let encoded = hex_encode(ssz::Encode::as_ssz_bytes(&hwp));
        assert_eq!(encoded, actual_hwp);
    }

    #[rstest::rstest]
    #[case(
        15539558,
        4702208,
        "15539558-cdf9ed89b0c43cda17398dc4da9cfc505e5ccd19f7c39e3b43474180f1051e01"
    )] // epoch 575
    #[case(
        15547621,
        4710400,
        "15547621-96a9313cd506e32893d46c82358569ad242bb32786bd5487833e0f77767aec2a"
    )] // epoch 576
    #[case(
        15555729,
        4718592,
        "15555729-c6fd396d54f61c6d0f1dd3653f81267b0378e9a0d638a229b24586d8fd0bc499"
    )] // epoch 577
    #[tokio::test]
    async fn historical_roots_proof_generation(
        #[case] block_number: u64,
        #[case] slot: u64,
        #[case] file_path: &str,
    ) {
        let test_vector = read_file_from_tests_submodule(format!(
            "tests/mainnet/history/headers_with_proof/block_proofs_bellatrix/beacon_block_proof-{file_path}.yaml"
        ))
        .unwrap();
        let test_vector: YamlValue = serde_yaml::from_str(&test_vector).unwrap();
        let expected_proof = BlockProofHistoricalRoots {
            beacon_block_proof: serde_yaml::from_value(test_vector["beacon_block_proof"].clone())
                .unwrap(),
            beacon_block_root: serde_yaml::from_value(test_vector["beacon_block_root"].clone())
                .unwrap(),
            execution_block_proof: serde_yaml::from_value(
                test_vector["execution_block_proof"].clone(),
            )
            .unwrap(),
            slot: serde_yaml::from_value(test_vector["slot"].clone()).unwrap(),
        };

        let test_assets_dir =
            format!("tests/mainnet/history/headers_with_proof/beacon_data/{block_number}");
        let historical_batch_raw =
            read_bytes_from_tests_submodule(format!("{test_assets_dir}/historical_batch.ssz",))
                .unwrap();
        let historical_batch = HistoricalBatch::from_ssz_bytes(&historical_batch_raw).unwrap();
        let block_raw =
            read_bytes_from_tests_submodule(format!("{test_assets_dir}/block.ssz",)).unwrap();
        let block = BeaconBlockBellatrix::from_ssz_bytes(&block_raw).unwrap();
        let actual_proof = build_block_proof_historical_roots(slot, historical_batch, block);

        assert_eq!(expected_proof, actual_proof);
    }

    #[rstest::rstest]
    #[case(17034870, 6209538)] // epoch 759
    #[case(17042287, 6217730)] // epoch 760
    #[case(17062257, 6238210)] // epoch 762
    #[tokio::test]
    async fn pre_deneb_historical_summaries_generation(
        #[case] block_number: u64,
        #[case] slot: u64,
    ) {
        let test_vector = read_file_from_tests_submodule(format!(
            "tests/mainnet/history/headers_with_proof/block_proofs_capella/beacon_block_proof-{block_number}.yaml",
        ))
        .unwrap();
        let test_vector: YamlValue = serde_yaml::from_str(&test_vector).unwrap();
        let expected_proof = BlockProofHistoricalSummaries {
            beacon_block_proof: serde_yaml::from_value(test_vector["beacon_block_proof"].clone())
                .unwrap(),
            beacon_block_root: serde_yaml::from_value(test_vector["beacon_block_root"].clone())
                .unwrap(),
            execution_block_proof: serde_yaml::from_value(
                test_vector["execution_block_proof"].clone(),
            )
            .unwrap(),
            slot: serde_yaml::from_value(test_vector["slot"].clone()).unwrap(),
        };

        let test_assets_dir =
            format!("tests/mainnet/history/headers_with_proof/beacon_data/{block_number}");
        let beacon_state_raw =
            read_bytes_from_tests_submodule(format!("{test_assets_dir}/beacon_state.ssz",))
                .unwrap();
        let beacon_state =
            BeaconState::from_ssz_bytes(&beacon_state_raw, ForkName::Capella).unwrap();
        let block_roots = beacon_state.as_capella().unwrap().block_roots.clone();
        let block_raw =
            read_bytes_from_tests_submodule(format!("{test_assets_dir}/block.ssz",)).unwrap();
        let block = BeaconBlockCapella::from_ssz_bytes(&block_raw).unwrap();
        let actual_proof = build_block_proof_historical_summaries(slot, block_roots, block);

        assert_eq!(expected_proof, actual_proof);
    }
}
