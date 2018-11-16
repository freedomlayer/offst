use im::hashmap::HashMap as ImHashMap;

use crypto::identity::{PublicKey, Signature};
use utils::int_convert::usize_to_u64;

use crate::friend::{FriendState, ChannelStatus, FriendMutation};
use crate::state::{FunderState, FunderMutation};
use crate::types::{RequestsStatus, FriendStatus, AddFriend, FriendMoveToken};
use crate::mutual_credit::types::{McBalance, McRequestsStatus};
use crate::token_channel::{TokenChannel, TcDirection, TcMutation}; 
use crate::liveness::Liveness;

#[derive(Clone, Debug)]
pub enum DirectionReport {
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug)]
pub enum FriendLivenessReport {
    Online,
    Offline,
}

#[derive(Clone, Debug)]
pub struct TcReport {
    pub direction: DirectionReport,
    pub balance: McBalance,
    pub requests_status: McRequestsStatus,
    pub num_local_pending_requests: u64,
    pub num_remote_pending_requests: u64,
}

#[derive(Clone, Debug)]
pub struct ResetTermsReport {
    reset_token: Signature,
    balance_for_reset: i128,
}

#[derive(Clone, Debug)]
pub struct ChannelInconsistentReport {
    pub local_reset_terms_balance: i128,
    pub opt_remote_reset_terms: Option<ResetTermsReport>,
}

#[derive(Clone, Debug)]
pub enum ChannelStatusReport {
    Inconsistent(ChannelInconsistentReport),
    Consistent(TcReport),
}

#[derive(Clone, Debug)]
pub struct FriendReport<A> {
    pub public_key: PublicKey,
    pub address: A, 
    pub name: String,
    // Last message signed by the remote side. 
    // Can be used as a proof for the last known balance.
    pub opt_last_incoming_move_token: Option<FriendMoveToken>,
    pub liveness: FriendLivenessReport, // is the friend online/offline?
    pub channel_status: ChannelStatusReport,
    pub wanted_remote_max_debt: u128,
    pub wanted_local_requests_status: RequestsStatus,
    pub num_pending_responses: u64,
    pub num_pending_requests: u64,
    // Pending operations to be sent to the token channel.
    pub status: FriendStatus,
    pub num_pending_user_requests: u64,
    // Request that the user has sent to this neighbor, 
    // but have not been processed yet. Bounded in size.
}

/// A FunderReport is a summary of a FunderState.
/// It contains the information the Funder exposes to the user apps of the Offst node.
#[derive(Debug)]
pub struct FunderReport<A: Clone> {
    pub friends: ImHashMap<PublicKey, FriendReport<A>>,
    pub num_ready_receipts: u64,
    pub local_public_key: PublicKey,

}

#[allow(unused)]
#[derive(Debug)]
pub enum FriendReportMutation<A> {
    SetFriendInfo((A, String)),
    SetChannelStatus(ChannelStatusReport),
    SetWantedRemoteMaxDebt(u128),
    SetWantedLocalRequestsStatus(RequestsStatus),
    SetNumPendingResponses(u64),
    SetNumPendingRequests(u64),
    SetFriendStatus(FriendStatus),
    SetNumPendingUserRequests(u64),
}

#[derive(Clone, Debug)]
pub struct AddFriendReport<A> {
    pub friend_public_key: PublicKey,
    pub address: A,
    pub name: String,
    pub balance: i128, // Initial balance
    pub channel_status: ChannelStatusReport,
}

#[allow(unused)]
#[derive(Debug)]
pub enum FunderReportMutation<A> {
    AddFriend(AddFriendReport<A>),
    RemoveFriend(PublicKey),
    FriendReportMutation((PublicKey, FriendReportMutation<A>)),
    SetNumReadyReceipts(u64),
}

fn create_token_channel_report(token_channel: &TokenChannel) -> TcReport {
    let direction = match token_channel.get_direction() {
        TcDirection::Incoming(_) => DirectionReport::Incoming,
        TcDirection::Outgoing(_) => DirectionReport::Outgoing,
    };
    let mutual_credit_state = token_channel.get_mutual_credit().state();
    TcReport {
        direction,
        balance: mutual_credit_state.balance.clone(),
        requests_status: mutual_credit_state.requests_status.clone(),
        num_local_pending_requests: usize_to_u64(mutual_credit_state.pending_requests.pending_local_requests.len()).unwrap(),
        num_remote_pending_requests: usize_to_u64(mutual_credit_state.pending_requests.pending_remote_requests.len()).unwrap(),
    }
}

fn create_channel_status_report<A: Clone>(channel_status: &ChannelStatus) -> ChannelStatusReport {
    match channel_status {
        ChannelStatus::Inconsistent(channel_inconsistent) => {
            let opt_remote_reset_terms = channel_inconsistent.opt_remote_reset_terms
                .clone()
                .map(|remote_reset_terms|
                    ResetTermsReport {
                        reset_token: remote_reset_terms.reset_token.clone(),
                        balance_for_reset: remote_reset_terms.balance_for_reset,
                    }
                );
            let channel_inconsistent_report = ChannelInconsistentReport {
                local_reset_terms_balance: channel_inconsistent.local_reset_terms.balance_for_reset,
                opt_remote_reset_terms,
            };
            ChannelStatusReport::Inconsistent(channel_inconsistent_report)
        },
        ChannelStatus::Consistent(token_channel) =>
            ChannelStatusReport::Consistent(create_token_channel_report(&token_channel)),
    }
}

fn create_friend_report<A: Clone>(friend_state: &FriendState<A>, friend_liveness: &FriendLivenessReport) -> FriendReport<A> {
    let channel_status = create_channel_status_report::<A>(&friend_state.channel_status);

    FriendReport {
        public_key: friend_state.remote_public_key.clone(),
        address: friend_state.remote_address.clone(),
        name: friend_state.name.clone(),
        opt_last_incoming_move_token: friend_state.channel_status.get_last_incoming_move_token(),
        liveness: friend_liveness.clone(),
        channel_status,
        wanted_remote_max_debt: friend_state.wanted_remote_max_debt,
        wanted_local_requests_status: friend_state.wanted_local_requests_status.clone(),
        num_pending_responses: usize_to_u64(friend_state.pending_responses.len()).unwrap(),
        num_pending_requests: usize_to_u64(friend_state.pending_requests.len()).unwrap(),
        status: friend_state.status.clone(),
        num_pending_user_requests: usize_to_u64(friend_state.pending_user_requests.len()).unwrap(),
    }
}

pub fn create_report<A: Clone>(funder_state: &FunderState<A>, liveness: &Liveness) -> FunderReport<A> {
    let mut friends = ImHashMap::new();
    for (friend_public_key, friend_state) in &funder_state.friends {
        let friend_liveness = match liveness.is_online(friend_public_key) {
            true => FriendLivenessReport::Online,
            false => FriendLivenessReport::Offline,
        };
        let friend_report = create_friend_report(&friend_state, &friend_liveness);
        friends.insert(friend_public_key.clone(), friend_report);
    }

    FunderReport {
        friends,
        num_ready_receipts: usize_to_u64(funder_state.ready_receipts.len()).unwrap(),
        local_public_key: funder_state.local_public_key.clone(),
    }

}

// TODO: How to add liveness mutation?
pub fn create_friend_report_mutation<A: Clone + 'static>(friend_mutation: &FriendMutation<A>,
                                           friend: &FriendState<A>) -> Option<FriendReportMutation<A>> {

    let mut friend_after = friend.clone();
    friend_after.mutate(friend_mutation);

    match friend_mutation {
        FriendMutation::TcMutation(tc_mutation) => {
            match tc_mutation {
                TcMutation::McMutation(_) |
                TcMutation::SetDirection(_) => {
                    let channel_status_report = create_channel_status_report::<A>(&friend_after.channel_status);
                    Some(FriendReportMutation::SetChannelStatus(channel_status_report))
                },
                TcMutation::SetTokenWanted => None,
            }
        },
        FriendMutation::SetInconsistent(_channel_inconsistent) =>
            Some(FriendReportMutation::SetChannelStatus(
                    create_channel_status_report::<A>(&friend_after.channel_status))),
        FriendMutation::SetWantedRemoteMaxDebt(wanted_remote_max_debt) =>
            Some(FriendReportMutation::SetWantedRemoteMaxDebt(*wanted_remote_max_debt)),
        FriendMutation::SetWantedLocalRequestsStatus(requests_status) => 
            Some(FriendReportMutation::SetWantedLocalRequestsStatus(requests_status.clone())),
        FriendMutation::PushBackPendingRequest(_request_send_funds) =>
            Some(FriendReportMutation::SetNumPendingRequests(
                    usize_to_u64(friend_after.pending_requests.len()).unwrap())),
        FriendMutation::PopFrontPendingRequest =>
            Some(FriendReportMutation::SetNumPendingRequests(
                    usize_to_u64(friend_after.pending_requests.len()).unwrap())),
        FriendMutation::PushBackPendingResponse(_response_op) =>
            Some(FriendReportMutation::SetNumPendingResponses(
                    usize_to_u64(friend_after.pending_responses.len()).unwrap())),
        FriendMutation::PopFrontPendingResponse => 
            Some(FriendReportMutation::SetNumPendingResponses(
                    usize_to_u64(friend_after.pending_responses.len()).unwrap())),
        FriendMutation::PushBackPendingUserRequest(_request_send_funds) =>
            Some(FriendReportMutation::SetNumPendingUserRequests(
                    usize_to_u64(friend_after.pending_user_requests.len()).unwrap())),
        FriendMutation::PopFrontPendingUserRequest => 
            Some(FriendReportMutation::SetNumPendingUserRequests(
                    usize_to_u64(friend_after.pending_user_requests.len()).unwrap())),
        FriendMutation::SetStatus(friend_status) => 
            Some(FriendReportMutation::SetFriendStatus(friend_status.clone())),
        FriendMutation::SetFriendInfo((address, name)) =>
            Some(FriendReportMutation::SetFriendInfo((address.clone(), name.clone()))),
        FriendMutation::LocalReset(_) |
        FriendMutation::RemoteReset(_) => 
            Some(FriendReportMutation::SetChannelStatus(
                    create_channel_status_report::<A>(&friend_after.channel_status))),
    }
}

/// Convert a FunderMutation to FunderReportMutation
/// FunderReportMutation are simpler than FunderMutations. They do not require reading the current
/// FunderReport. However, FunderMutations sometimes require access to the current funder_state to
/// make sense. Therefore we require that this function takes FunderState too.
///
/// In the future if we simplify Funder's mutations, we might be able discard the `funder_state`
/// argument here.
pub fn create_funder_report_mutation<A: Clone + 'static>(funder_mutation: &FunderMutation<A>,
                                           funder_state: &FunderState<A>) -> Option<FunderReportMutation<A>> {

    let mut funder_state_after = funder_state.clone();
    funder_state_after.mutate(funder_mutation);
    match funder_mutation {
        FunderMutation::FriendMutation((public_key, friend_mutation)) => {
            let friend = funder_state.friends.get(public_key).unwrap();
            let friend_report_mutation = create_friend_report_mutation(&friend_mutation, &friend)?;
            Some(FunderReportMutation::FriendReportMutation((public_key.clone(), friend_report_mutation)))
        },
        FunderMutation::AddFriend(add_friend) => {
            let friend_after = funder_state_after.friends.get(&add_friend.friend_public_key).unwrap();
            let add_friend_report = AddFriendReport {
                friend_public_key: add_friend.friend_public_key.clone(),
                address: add_friend.address.clone(),
                name: add_friend.name.clone(),
                balance: add_friend.balance.clone(), // Initial balance
                channel_status: create_channel_status_report::<A>(&friend_after.channel_status),
            };
            Some(FunderReportMutation::AddFriend(add_friend_report))
        },
        FunderMutation::RemoveFriend(friend_public_key) => {
            Some(FunderReportMutation::RemoveFriend(friend_public_key.clone()))
        },
        FunderMutation::AddReceipt((_uid, _receipt)) => {
            if funder_state_after.ready_receipts.len() != funder_state.ready_receipts.len() {
                Some(FunderReportMutation::SetNumReadyReceipts(usize_to_u64(funder_state.ready_receipts.len()).unwrap()))
            } else {
                None
            }
        },
        FunderMutation::RemoveReceipt(_uid) => {
            if funder_state_after.ready_receipts.len() != funder_state.ready_receipts.len() {
                Some(FunderReportMutation::SetNumReadyReceipts(usize_to_u64(funder_state.ready_receipts.len()).unwrap()))
            } else {
                None
            }
        },
    }
}


impl<A: Clone> FunderReport<A> {
    fn mutate(&mut self, mutation: &FunderReportMutation<A>) {
        match mutation {
            FunderReportMutation::AddFriend(add_friend_report) => {
                // TODO: AddFriend Should include information about how to build channel_status
                unimplemented!();
            },
            FunderReportMutation::RemoveFriend(friend_public_key) => {
                let _ = self.friends.remove(&friend_public_key);
            },
            FunderReportMutation::FriendReportMutation((friend_public_key, friend_report_mutation)) => unimplemented!(),
            FunderReportMutation::SetNumReadyReceipts(num_ready_receipts) => {
                self.num_ready_receipts = *num_ready_receipts;
            },
        }
    }
}

