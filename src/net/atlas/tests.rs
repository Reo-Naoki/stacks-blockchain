use super::download::{
    AttachmentRequest, AttachmentsBatch, AttachmentsBatchStateContext, AttachmentsInventoryRequest,
    BatchedRequestsResult, ReliabilityReport,
};
use super::{Attachment, AttachmentInstance};
use chainstate::burn::{BlockHeaderHash, ConsensusHash};
use chainstate::stacks::db::StacksChainState;
use chainstate::stacks::{StacksBlockHeader, StacksBlockId};
use net::connection::ConnectionOptions;
use net::{
    AttachmentPage, GetAttachmentsInvResponse, HttpResponseMetadata, HttpResponseType, HttpVersion,
    PeerHost, Requestable,
};
use util::hash::Hash160;
use vm::representations::UrlString;
use vm::types::QualifiedContractIdentifier;

use std::collections::{BinaryHeap, HashMap};
use std::convert::TryFrom;

fn new_attachment_from(content: &str) -> Attachment {
    Attachment {
        content: content.as_bytes().to_vec(),
    }
}

fn new_attachment_instance_from(
    attachment: &Attachment,
    position_in_page: u32,
    page_index: u32,
    block_height: u64,
) -> AttachmentInstance {
    AttachmentInstance {
        content_hash: attachment.hash().clone(),
        page_index,
        position_in_page,
        block_height,
        consensus_hash: ConsensusHash::empty(),
        metadata: "".to_string(),
        contract_id: QualifiedContractIdentifier::transient(),
        block_header_hash: BlockHeaderHash([0x00; 32]),
    }
}

fn new_attachments_batch_from(
    attachment_instances: Vec<AttachmentInstance>,
    retry_count: u32,
) -> AttachmentsBatch {
    let mut attachments_batch = AttachmentsBatch::new();
    for attachment_instance in attachment_instances.iter() {
        attachments_batch.track_attachment(&attachment_instance);
    }
    for _ in 0..retry_count {
        attachments_batch.bump_retry_count();
    }
    attachments_batch
}

fn new_peers(peers: Vec<(&str, u32, u32)>) -> HashMap<UrlString, ReliabilityReport> {
    let mut new_peers = HashMap::new();
    for (url, req_sent, req_success) in peers {
        let url = UrlString::try_from(format!("{}", url).as_str()).unwrap();
        new_peers.insert(url, ReliabilityReport::new(req_sent, req_success));
    }
    new_peers
}

fn new_attachment_request(
    sources: Vec<(&str, u32, u32)>,
    content_hash: &Hash160,
) -> AttachmentRequest {
    let sources = {
        let mut s = HashMap::new();
        for (url, req_sent, req_success) in sources {
            let url = UrlString::try_from(format!("{}", url).as_str()).unwrap();
            s.insert(url, ReliabilityReport::new(req_sent, req_success));
        }
        s
    };
    AttachmentRequest {
        sources,
        content_hash: content_hash.clone(),
    }
}

fn new_attachments_inventory_request(
    url: &str,
    pages: Vec<u32>,
    block_height: u64,
    req_sent: u32,
    req_success: u32,
) -> AttachmentsInventoryRequest {
    let url = UrlString::try_from(format!("{}", url).as_str()).unwrap();
    AttachmentsInventoryRequest {
        url,
        block_height,
        pages,
        contract_id: QualifiedContractIdentifier::transient(),
        consensus_hash: ConsensusHash::empty(),
        block_header_hash: BlockHeaderHash([0x00; 32]),
        reliability_report: ReliabilityReport::new(req_sent, req_success),
    }
}

fn new_attachments_inventory_response(pages: Vec<(u32, Vec<u8>)>) -> HttpResponseType {
    let md = HttpResponseMetadata::new(HttpVersion::Http11, 1, None, true);
    let pages = pages
        .into_iter()
        .map(|(index, inventory)| AttachmentPage { index, inventory })
        .collect();
    let response = GetAttachmentsInvResponse {
        block_id: StacksBlockId([0u8; 32]),
        pages,
    };
    HttpResponseType::GetAttachmentsInv(md, response)
}

#[test]
fn test_attachment_instance_parsing() {
    use util::hash::MerkleHashFunc;
    use vm;

    let contract_id = QualifiedContractIdentifier::transient();
    let block_height = 0;
    let consensus_hash = ConsensusHash::empty();
    let block_header_hash = BlockHeaderHash([0x00; 32]);

    let value_1 = vm::execute(
        r#"
    {
        attachment: {
            position-in-page: u1,
            page-index: u1,
            hash: 0x,
            metadata: {
                name: "stacks"
            }
        }
    }
    "#,
    )
    .unwrap()
    .unwrap();
    let attachment_instance_1 = AttachmentInstance::try_new_from_value(
        &value_1,
        &contract_id,
        &consensus_hash,
        block_header_hash.clone(),
        block_height,
    )
    .unwrap();
    assert_eq!(attachment_instance_1.page_index, 1);
    assert_eq!(attachment_instance_1.position_in_page, 1);
    assert_eq!(attachment_instance_1.content_hash, Hash160::empty());

    let value_2 = vm::execute(
        r#"
    {
        attachment: {
            position-in-page: u2,
            page-index: u2,
            hash: 0xd37581093088f5237a8dc885f38c231e42389cb2,
            metadata: {
                name: "stacks"
            }
        }
    }
    "#,
    )
    .unwrap()
    .unwrap();
    let attachment_instance_2 = AttachmentInstance::try_new_from_value(
        &value_2,
        &contract_id,
        &consensus_hash,
        block_header_hash.clone(),
        block_height,
    )
    .unwrap();
    assert_eq!(attachment_instance_2.page_index, 2);
    assert_eq!(attachment_instance_2.position_in_page, 2);
    assert_eq!(
        attachment_instance_2.content_hash,
        Hash160::from_hex("d37581093088f5237a8dc885f38c231e42389cb2").unwrap()
    );

    let value_3 = vm::execute(
        r#"
    {
        attachment: {
            position-in-page: u3,
            page-index: u3,
            hash: 0xd37581093088f5237a8dc885f38c231e42389cb2
        }
    }
    "#,
    )
    .unwrap()
    .unwrap();
    let attachment_instance_3 = AttachmentInstance::try_new_from_value(
        &value_3,
        &contract_id,
        &consensus_hash,
        block_header_hash.clone(),
        block_height,
    )
    .unwrap();
    assert_eq!(attachment_instance_3.page_index, 3);
    assert_eq!(attachment_instance_3.position_in_page, 3);
    assert_eq!(
        attachment_instance_3.content_hash,
        Hash160::from_hex("d37581093088f5237a8dc885f38c231e42389cb2").unwrap()
    );

    let values = [
        vm::execute(
            r#"
    {
        attachment: {
            position-in-page: 1,
            page-index: u1,
            hash: 0x
        }
    }
    "#,
        )
        .unwrap()
        .unwrap(),
        vm::execute(
            r#"
    {
        attachment: {
            position-in-page: u1,
            page-index: u1,
            hash: 0x1323
        }
    }
    "#,
        )
        .unwrap()
        .unwrap(),
        vm::execute(
            r#"
    {
        attachment: {
            position-in-page: u1,
            pages-index: u1,
            hash: 0x
        }
    }
    "#,
        )
        .unwrap()
        .unwrap(),
    ];

    for value in values.iter() {
        AttachmentInstance::try_new_from_value(
            &value,
            &contract_id,
            &consensus_hash,
            block_header_hash.clone(),
            block_height,
        )
        .unwrap_err();
    }
}

#[test]
fn test_attachments_batch_ordering() {
    // Ensuring that when batches are being queued, we are correctly dequeueing, based on the following priorities:
    // 1) the batch that has been the least retried,
    // 2) if tie, the batch that will lead to the maximum number of downloads,
    // 3) if tie, the most oldest batch

    // Batch 1: 4 attachments, never tried, emitted at block #1
    let attachments_batch_1 = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&new_attachment_from("facade01"), 1, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade02"), 2, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade03"), 3, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade04"), 4, 1, 1),
        ],
        0,
    );

    // Batch 2: 5 attachments, never tried, emitted at block #2
    let attachments_batch_2 = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&new_attachment_from("facade11"), 1, 1, 2),
            new_attachment_instance_from(&new_attachment_from("facade12"), 2, 1, 2),
            new_attachment_instance_from(&new_attachment_from("facade13"), 3, 1, 2),
            new_attachment_instance_from(&new_attachment_from("facade14"), 4, 1, 2),
            new_attachment_instance_from(&new_attachment_from("facade15"), 5, 1, 2),
        ],
        0,
    );

    // Batch 3: 4 attachments, tried once, emitted at block #3
    let attachments_batch_3 = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&new_attachment_from("facade21"), 1, 2, 3),
            new_attachment_instance_from(&new_attachment_from("facade22"), 2, 2, 3),
            new_attachment_instance_from(&new_attachment_from("facade23"), 3, 2, 3),
            new_attachment_instance_from(&new_attachment_from("facade24"), 4, 2, 3),
        ],
        1,
    );

    // Batch 1: 4 attachments, never tried, emitted at block #4
    let attachments_batch_4 = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&new_attachment_from("facade31"), 1, 3, 4),
            new_attachment_instance_from(&new_attachment_from("facade32"), 2, 3, 4),
            new_attachment_instance_from(&new_attachment_from("facade33"), 3, 3, 4),
            new_attachment_instance_from(&new_attachment_from("facade34"), 4, 3, 4),
        ],
        0,
    );

    let mut priority_queue = BinaryHeap::new();
    priority_queue.push(attachments_batch_1.clone());
    priority_queue.push(attachments_batch_2.clone());
    priority_queue.push(attachments_batch_3.clone());
    priority_queue.push(attachments_batch_4.clone());

    // According to the rules above, the expected order is:
    // 1) Batch 2 (tie on retry count with 1 and 4 -> attachments count)
    // 2) Batch 1 (tie on retry count with 4 -> tie on attachments count 4 -> block height)
    // 3) Batch 4 (retry count)
    // 4) Batch 3
    assert_eq!(priority_queue.pop().unwrap(), attachments_batch_2);
    assert_eq!(priority_queue.pop().unwrap(), attachments_batch_1);
    assert_eq!(priority_queue.pop().unwrap(), attachments_batch_4);
    assert_eq!(priority_queue.pop().unwrap(), attachments_batch_3);
}

#[test]
fn test_attachments_inventory_requests_ordering() {
    // Ensuring that when we're flooding a set of peers with GetAttachmentsInventory requests, the order is based on the following rules:
    // 1) Nodes with highest ratio successful requests / total requests
    // 2) if tie, biggest number of total requests
    let attachments_inventory_1_request =
        new_attachments_inventory_request("http://localhost:20443", vec![0, 1], 1, 0, 0);

    let attachments_inventory_2_request =
        new_attachments_inventory_request("http://localhost:30443", vec![0, 1], 1, 2, 1);

    let attachments_inventory_3_request =
        new_attachments_inventory_request("http://localhost:40443", vec![0, 1], 1, 2, 2);

    let attachments_inventory_4_request =
        new_attachments_inventory_request("http://localhost:50443", vec![0, 1], 1, 4, 4);

    let mut priority_queue = BinaryHeap::new();
    priority_queue.push(attachments_inventory_2_request.clone());
    priority_queue.push(attachments_inventory_1_request.clone());
    priority_queue.push(attachments_inventory_4_request.clone());
    priority_queue.push(attachments_inventory_3_request.clone());

    // According to the rules above, the expected order is:
    // 1) Request 4
    // 2) Request 3
    // 2) Request 2
    // 3) Request 1
    assert_eq!(
        priority_queue.pop().unwrap(),
        attachments_inventory_4_request
    );
    assert_eq!(
        priority_queue.pop().unwrap(),
        attachments_inventory_3_request
    );
    assert_eq!(
        priority_queue.pop().unwrap(),
        attachments_inventory_2_request
    );
    assert_eq!(
        priority_queue.pop().unwrap(),
        attachments_inventory_1_request
    );
}

#[test]
fn test_attachment_requests_ordering() {
    // Ensuring that when we're downloading some attachments, the order is based on the following rules:
    // 1) attachments that are the least available
    // 2) if tie, starting with the most reliable peer
    let attachment_1 = new_attachment_from("facade01");
    let attachment_2 = new_attachment_from("facade02");
    let attachment_3 = new_attachment_from("facade03");
    let attachment_4 = new_attachment_from("facade04");

    let attachment_1_request = new_attachment_request(
        vec![
            ("http://localhost:20443", 2, 2),
            ("http://localhost:40443", 0, 1),
        ],
        &attachment_1.hash(),
    );

    let attachment_2_request = new_attachment_request(
        vec![
            ("http://localhost:20443", 2, 2),
            ("http://localhost:40443", 0, 1),
            ("http://localhost:30443", 0, 1),
        ],
        &attachment_2.hash(),
    );

    let attachment_3_request =
        new_attachment_request(vec![("http://localhost:30443", 0, 1)], &attachment_3.hash());

    let attachment_4_request =
        new_attachment_request(vec![("http://localhost:50443", 4, 4)], &attachment_4.hash());

    let mut priority_queue = BinaryHeap::new();
    priority_queue.push(attachment_1_request.clone());
    priority_queue.push(attachment_3_request.clone());
    priority_queue.push(attachment_4_request.clone());
    priority_queue.push(attachment_2_request.clone());

    // According to the rules above, the expected order is:
    // 1) Request 4
    // 2) Request 3
    // 3) Request 1
    // 4) Request 2
    assert_eq!(priority_queue.pop().unwrap(), attachment_4_request);
    assert_eq!(priority_queue.pop().unwrap(), attachment_3_request);
    assert_eq!(priority_queue.pop().unwrap(), attachment_1_request);
    assert_eq!(priority_queue.pop().unwrap(), attachment_2_request);
}

#[test]
fn test_attachments_batch_constructs() {
    let attachment_instance_1 =
        new_attachment_instance_from(&new_attachment_from("facade11"), 1, 1, 1);
    let attachment_instance_2 =
        new_attachment_instance_from(&new_attachment_from("facade12"), 2, 1, 1);
    let attachment_instance_3 =
        new_attachment_instance_from(&new_attachment_from("facade13"), 3, 1, 1);
    let attachment_instance_4 =
        new_attachment_instance_from(&new_attachment_from("facade14"), 4, 1, 1);
    let attachment_instance_5 =
        new_attachment_instance_from(&new_attachment_from("facade15"), 1, 2, 1);

    let mut attachments_batch = AttachmentsBatch::new();
    attachments_batch.track_attachment(&attachment_instance_1);
    attachments_batch.track_attachment(&attachment_instance_2);
    attachments_batch.track_attachment(&attachment_instance_3);
    attachments_batch.track_attachment(&attachment_instance_4);
    attachments_batch.track_attachment(&attachment_instance_5);

    let default_contract_id = QualifiedContractIdentifier::transient();

    assert_eq!(attachments_batch.attachments_instances_count(), 5);
    assert_eq!(
        attachments_batch
            .get_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        2
    );

    attachments_batch.resolve_attachment(&attachment_instance_5.content_hash);
    assert_eq!(attachments_batch.attachments_instances_count(), 4);
    assert_eq!(
        attachments_batch
            .get_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        1
    );

    // should be idempotent
    attachments_batch.resolve_attachment(&attachment_instance_5.content_hash);
    assert_eq!(attachments_batch.attachments_instances_count(), 4);

    attachments_batch.resolve_attachment(&attachment_instance_2.content_hash);
    attachments_batch.resolve_attachment(&attachment_instance_3.content_hash);
    attachments_batch.resolve_attachment(&attachment_instance_4.content_hash);
    assert_eq!(attachments_batch.has_fully_succeed(), false);

    attachments_batch.resolve_attachment(&attachment_instance_1.content_hash);
    assert_eq!(attachments_batch.has_fully_succeed(), true);
    assert_eq!(
        attachments_batch
            .get_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        0
    );
}


#[test]
fn test_attachments_batch_pages() {
    let attachment_instance_1 =
        new_attachment_instance_from(&new_attachment_from("facade11"), 1, 1, 1);
    let attachment_instance_2 =
        new_attachment_instance_from(&new_attachment_from("facade12"), 2, 2, 1);
    let attachment_instance_3 =
        new_attachment_instance_from(&new_attachment_from("facade13"), 3, 3, 1);
    let attachment_instance_4 =
        new_attachment_instance_from(&new_attachment_from("facade14"), 4, 4, 1);
    let attachment_instance_5 =
        new_attachment_instance_from(&new_attachment_from("facade15"), 1, 5, 1);
    let attachment_instance_6 =
        new_attachment_instance_from(&new_attachment_from("facade16"), 1, 6, 1);
    let attachment_instance_7 =
        new_attachment_instance_from(&new_attachment_from("facade17"), 2, 7, 1);
    let attachment_instance_8 =
        new_attachment_instance_from(&new_attachment_from("facade18"), 3, 8, 1);
    let attachment_instance_9 =
        new_attachment_instance_from(&new_attachment_from("facade19"), 4, 9, 1);
    let attachment_instance_10 =
        new_attachment_instance_from(&new_attachment_from("facade20"), 1, 10, 1);

    let mut attachments_batch = AttachmentsBatch::new();
    attachments_batch.track_attachment(&attachment_instance_1);
    attachments_batch.track_attachment(&attachment_instance_2);
    attachments_batch.track_attachment(&attachment_instance_3);
    attachments_batch.track_attachment(&attachment_instance_4);
    attachments_batch.track_attachment(&attachment_instance_5);
    attachments_batch.track_attachment(&attachment_instance_6);
    attachments_batch.track_attachment(&attachment_instance_7);
    attachments_batch.track_attachment(&attachment_instance_8);
    attachments_batch.track_attachment(&attachment_instance_9);
    attachments_batch.track_attachment(&attachment_instance_10);

    let default_contract_id = QualifiedContractIdentifier::transient();

    assert_eq!(attachments_batch.attachments_instances_count(), 10);
    assert_eq!(
        attachments_batch
            .get_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        10
    );

    assert_eq!(
        attachments_batch
            .get_paginated_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        2
    );

    attachments_batch.resolve_attachment(&attachment_instance_1.content_hash);
    attachments_batch.resolve_attachment(&attachment_instance_2.content_hash);
    attachments_batch.resolve_attachment(&attachment_instance_3.content_hash);

    // Assuming MAX_ATTACHMENT_INV_PAGES_PER_REQUEST = 8
    assert_eq!(
        attachments_batch
            .get_paginated_missing_pages_for_contract_id(&default_contract_id)
            .len(),
        1
    );
}

#[test]
fn test_downloader_context_attachment_inventories_requests() {
    let localhost = PeerHost::from_host_port("127.0.0.1".to_string(), 1024);
    let attachments_batch = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&new_attachment_from("facade01"), 1, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade02"), 2, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade03"), 3, 1, 1),
            new_attachment_instance_from(&new_attachment_from("facade04"), 1, 2, 1),
        ],
        0,
    );
    let peers = new_peers(vec![
        ("http://localhost:20443", 2, 2),
        ("http://localhost:30443", 3, 3),
        ("http://localhost:40443", 0, 0),
    ]);
    let context =
        AttachmentsBatchStateContext::new(attachments_batch, peers, &ConnectionOptions::default());

    let mut request_queue = context.get_prioritized_attachments_inventory_requests();
    let request = request_queue.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    assert_eq!(&**request.get_url(), "http://localhost:30443");
    assert_eq!(
        request_type.request_path(),
        "/v2/attachments/inv?pages_indexes=1,2"
    );

    let request = request_queue.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    assert_eq!(&**request.get_url(), "http://localhost:20443");
    assert_eq!(
        request_type.request_path(),
        "/v2/attachments/inv?pages_indexes=1,2"
    );

    let request = request_queue.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    assert_eq!(&**request.get_url(), "http://localhost:40443");
    assert_eq!(
        request_type.request_path(),
        "/v2/attachments/inv?pages_indexes=1,2"
    );
}

#[test]
fn test_downloader_context_attachment_requests() {
    let attachment_1 = new_attachment_from("facade01");
    let attachment_2 = new_attachment_from("facade02");
    let attachment_3 = new_attachment_from("facade03");
    let attachment_4 = new_attachment_from("facade04");

    let localhost = PeerHost::from_host_port("127.0.0.1".to_string(), 1024);
    let attachments_batch = new_attachments_batch_from(
        vec![
            new_attachment_instance_from(&attachment_1, 0, 1, 1),
            new_attachment_instance_from(&attachment_2, 1, 1, 1),
            new_attachment_instance_from(&attachment_3, 2, 1, 1),
            new_attachment_instance_from(&attachment_4, 0, 2, 1),
        ],
        0,
    );
    let peers = new_peers(vec![
        ("http://localhost:20443", 4, 4),
        ("http://localhost:30443", 3, 3),
        ("http://localhost:40443", 2, 2),
        ("http://localhost:50443", 1, 1),
    ]);
    let context =
        AttachmentsBatchStateContext::new(attachments_batch, peers, &ConnectionOptions::default());

    let mut inventories_requests = context.get_prioritized_attachments_inventory_requests();
    let mut inventories_results = BatchedRequestsResult::empty();

    let request = inventories_requests.pop().unwrap();
    let peer_url_1 = request.get_url().clone();
    let request = inventories_requests.pop().unwrap();
    let peer_url_2 = request.get_url().clone();
    let request = inventories_requests.pop().unwrap();
    let peer_url_3 = request.get_url().clone();
    let request = inventories_requests.pop().unwrap();
    let peer_url_4 = request.get_url().clone();
    let mut responses = HashMap::new();

    let response_1 =
        new_attachments_inventory_response(vec![(1, vec![1, 1, 1]), (2, vec![0, 0, 0])]);
    responses.insert(peer_url_1.clone(), Some(response_1));

    let response_2 =
        new_attachments_inventory_response(vec![(1, vec![1, 1, 1]), (2, vec![0, 0, 0])]);
    responses.insert(peer_url_2.clone(), Some(response_2));

    let response_3 =
        new_attachments_inventory_response(vec![(1, vec![0, 1, 1]), (2, vec![1, 0, 0])]);
    responses.insert(peer_url_3.clone(), Some(response_3));
    responses.insert(peer_url_4, None);

    inventories_results.succeeded.insert(request, responses);

    let context = context.extend_with_inventories(&mut inventories_results);

    let mut attachments_requests = context.get_prioritized_attachments_requests();

    let request = attachments_requests.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    // Attachment 4 is the rarest resource
    assert_eq!(
        request_type.request_path(),
        format!("/v2/attachments/{}", attachment_4.hash())
    );
    // Peer 1 is the only peer showing Attachment 4 as being available in its inventory
    assert_eq!(request.get_url(), &peer_url_3);

    let request = attachments_requests.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    // Attachment 1 is the 2nd rarest resource
    assert_eq!(
        request_type.request_path(),
        format!("/v2/attachments/{}", attachment_1.hash())
    );
    // Both Peer 1 and Peer 2 could serve Attachment 1, but Peer 1 has a better history
    assert_eq!(request.get_url(), &peer_url_1);

    // The 2 last requests can be served by Peer 1, 2 and 3, but will be served in random
    // order by Peer 1 (best score).
    let request = attachments_requests.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    assert_eq!(request.get_url(), &peer_url_1);

    let request = attachments_requests.pop().unwrap();
    let request_type = request.make_request_type(localhost.clone());
    assert_eq!(request.get_url(), &peer_url_1);
}

#[test]
fn test_downloader_dns_state_machine() {}

#[test]
fn test_downloader_batched_requests_state_machine() {}

// todo(ludo): write tests around the fact that one hash can exist multiple inside the same fork as well.