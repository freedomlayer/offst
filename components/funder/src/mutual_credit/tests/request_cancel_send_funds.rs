use std::convert::TryFrom;

use futures::channel::mpsc;
use futures::task::SpawnExt;
use futures::{FutureExt, TryFutureExt};

use common::test_executor::TestExecutor;

use crypto::hash_lock::HashLock;
use crypto::identity::{Identity, SoftwareEd25519Identity};
use crypto::rand::RandGen;
use crypto::test_utils::DummyRandom;

use proto::crypto::{HashResult, HmacResult, PlainLock, PrivateKey, PublicKey, Uid};
use proto::funder::messages::{
    CancelSendFundsOp, Currency, FriendTcOp, FriendsRoute, RequestSendFundsOp,
};

use crate::mutual_credit::tests::utils::{mc_server, MutualCredit};
use crate::mutual_credit::types::McTransaction;

use crate::mutual_credit::incoming::process_operations_list;
use crate::mutual_credit::outgoing::queue_operation;

async fn task_request_cancel_send_funds(test_executor: TestExecutor) {
    let currency = Currency::try_from("FST".to_owned()).unwrap();

    let mut rng = DummyRandom::new(&[1u8]);
    let private_key = PrivateKey::rand_gen(&mut rng);
    let identity = SoftwareEd25519Identity::from_private_key(&private_key).unwrap();
    let public_key_b = identity.get_public_key();

    let local_public_key = PublicKey::from(&[0xaa; PublicKey::len()]);
    let remote_public_key = public_key_b.clone();
    let balance = 0;

    let mutual_credit =
        MutualCredit::new(&local_public_key, &remote_public_key, &currency, balance);
    let (sender, receiver) = mpsc::channel(0);
    test_executor
        .spawn(
            mc_server(mutual_credit, receiver)
                .map_err(|e| warn!("mc_server closed with error: {:?}", e))
                .map(|_| ()),
        )
        .unwrap();
    let mut mc_transaction = McTransaction::new(sender);

    // -----[RequestSendFunds]--------
    // -----------------------------
    let request_id = Uid::from(&[3; Uid::len()]);
    let route = FriendsRoute {
        public_keys: vec![
            PublicKey::from(&[0xaa; PublicKey::len()]),
            public_key_b.clone(),
            PublicKey::from(&[0xcc; PublicKey::len()]),
        ],
    };
    let invoice_hash = HashResult::from(&[0; HashResult::len()]);
    let src_plain_lock = PlainLock::from(&[1; PlainLock::len()]);
    let hmac = HmacResult::from(&[2; HmacResult::len()]);

    let request_send_funds = RequestSendFundsOp {
        request_id: request_id.clone(),
        src_hashed_lock: src_plain_lock.hash_lock(),
        route,
        dest_payment: 10,
        total_dest_payment: 10,
        invoice_hash,
        hmac,
        left_fees: 5,
    };

    queue_operation(
        &mut mc_transaction,
        FriendTcOp::RequestSendFunds(request_send_funds),
        &currency,
        &local_public_key,
    )
    .await
    .unwrap();

    let mc_balance = mc_transaction.get_balance().await.unwrap();
    assert_eq!(mc_balance.balance, 0);
    assert_eq!(mc_balance.local_pending_debt, 10 + 5);
    assert_eq!(mc_balance.remote_pending_debt, 0);

    // -----[CancelSendFunds]--------
    // ------------------------------
    let cancel_send_funds = CancelSendFundsOp { request_id };

    process_operations_list(
        &mut mc_transaction,
        vec![FriendTcOp::CancelSendFunds(cancel_send_funds)],
        &currency,
        &remote_public_key,
        100,
    )
    .await
    .unwrap();

    let mc_balance = mc_transaction.get_balance().await.unwrap();
    assert_eq!(mc_balance.balance, 0);
    assert_eq!(mc_balance.local_pending_debt, 0);
    assert_eq!(mc_balance.remote_pending_debt, 0);
}

#[test]
fn test_request_cancel_send_funds() {
    let test_executor = TestExecutor::new();
    let res = test_executor.run(task_request_cancel_send_funds(test_executor.clone()));
    assert!(res.is_output());
}