use std::convert::TryFrom;

use futures::prelude::{async, await};

use num_bigint::BigUint;
use num_traits::ToPrimitive;

use ring::rand::SecureRandom;

use crypto::rand_values::RandValue;
use crypto::identity::{PublicKey, Signature};
use crypto::uid::Uid;

use utils::safe_arithmetic::SafeArithmetic;


use proto::funder::ChannelToken;

use super::super::token_channel::incoming::{IncomingResponseSendFunds, 
    IncomingFailureSendFunds, IncomingFunds};
use super::super::token_channel::outgoing::{OutgoingTokenChannel, QueueOperationFailure,
    QueueOperationError};
use super::super::token_channel::directional::{ReceiveMoveTokenOutput, ReceiveMoveTokenError, 
    DirectionalMutation, MoveTokenDirection, MoveTokenReceived};
use super::{MutableFunderHandler, FunderTask, FriendMessage,
            RequestReceived, ResponseReceived, FailureReceived};
use super::super::types::{FriendTcOp, RequestSendFunds, 
    ResponseSendFunds, FailureSendFunds, 
    FriendMoveToken};
use super::super::token_channel::types::FriendMoveTokenInner;
use super::super::state::FunderMutation;
use super::super::friend::{FriendState, FriendMutation, OutgoingInconsistency, IncomingInconsistency, ResetTerms};

use super::super::signature_buff::create_failure_signature_buffer;
use super::super::types::{FunderFreezeLink, PkPairPosition, PendingFriendRequest, Ratio};


// Approximate maximum size of a MOVE_TOKEN message.
// TODO: Where to put this constant? Do we have more like this one?
const MAX_MOVE_TOKEN_LENGTH: usize = 0x1000;


#[allow(unused)]
pub struct FriendInconsistencyError {
    opt_ack: Option<ChannelToken>,
    current_token: ChannelToken,
    balance_for_reset: i128,
}


#[allow(unused)]
pub enum IncomingFriendFunds {
    MoveToken(FriendMoveToken),
    InconsistencyError(FriendInconsistencyError),
}

pub enum HandleFriendError {
}


#[allow(unused)]
impl<A: Clone, R: SecureRandom + 'static> MutableFunderHandler<A,R> {

    fn get_friend(&self, friend_public_key: &PublicKey) -> Option<&FriendState<A>> {
        self.state.get_friends().get(&friend_public_key)
    }

    /// Find the originator of a pending local request.
    /// This should be a pending remote request at some other friend.
    /// Returns the public key of a friend together with the channel_index of a
    /// token channel. If we are the origin of this request, the function return None.
    ///
    /// TODO: We need to change this search to be O(1) in the future. Possibly by maintaining a map
    /// between request_id and (friend_public_key, friend).
    fn find_request_origin(&self, request_id: &Uid) -> Option<&PublicKey> {
        for (friend_public_key, friend) in self.state.get_friends() {
            if friend.directional
                .token_channel
                .state()
                .pending_requests
                .pending_remote_requests
                .contains_key(request_id) {
                    return Some(friend_public_key)
            }
        }
        None
    }

    /// Create a (signed) failure message for a given request_id.
    /// We are the reporting_public_key for this failure message.
    #[async]
    fn create_failure_message(mut self, pending_local_request: PendingFriendRequest) 
        -> Result<(Self, FailureSendFunds), HandleFriendError> {

        let rand_nonce = RandValue::new(&*self.rng);
        let local_public_key = self.state.get_local_public_key().clone();

        let failure_send_funds = FailureSendFunds {
            request_id: pending_local_request.request_id,
            reporting_public_key: local_public_key.clone(),
            rand_nonce,
            signature: Signature::zero(),
        };
        // TODO: Add default() implementation for Signature
        
        let mut failure_signature_buffer = create_failure_signature_buffer(
                                            &failure_send_funds,
                                            &pending_local_request);

        let signature = await!(self.security_module_client.request_signature(failure_signature_buffer))
            .expect("Failed to create a signature!");

        Ok((self, FailureSendFunds {
            request_id: pending_local_request.request_id,
            reporting_public_key: local_public_key,
            rand_nonce,
            signature,
        }))
    }


    #[async]
    fn cancel_local_pending_requests(mut self, 
                                     friend_public_key: PublicKey) -> Result<Self, HandleFriendError> {

        let friend = self.get_friend(&friend_public_key).unwrap();

        // Mark all pending requests to this friend as errors.  
        // As the token channel is being reset, we can be sure we will never obtain a response
        // for those requests.
        let pending_local_requests = friend.directional
            .token_channel
            .state()
            .pending_requests
            .pending_local_requests
            .clone();

        let local_public_key = self.state.get_local_public_key().clone();
        let mut fself = self;
        // Prepare a list of all remote requests that we need to cancel:
        for (local_request_id, pending_local_request) in pending_local_requests {
            let opt_origin_public_key = fself.find_request_origin(&local_request_id);
            let origin_public_key = match opt_origin_public_key {
                Some(origin_public_key) => origin_public_key,
                None => continue,
            };

            let (new_fself, failure_send_funds) = await!(fself.create_failure_message(pending_local_request))?;
            fself = new_fself;

            let failure_op = FriendTcOp::FailureSendFunds(failure_send_funds);
            let friend_mutation = FriendMutation::PushBackPendingOperation(failure_op);
            let messenger_mutation = FunderMutation::FriendMutation((origin_public_key.clone(), friend_mutation));
            fself.apply_mutation(messenger_mutation);
        }
        Ok(fself)
   }


    /// Check if channel reset is required (Remove side used the RESET token)
    /// If so, reset the channel.
    #[async]
    fn check_reset_channel(mut self, 
                           friend_public_key: PublicKey,
                           new_token: ChannelToken) -> Result<Self, HandleFriendError> {
        // Check if incoming message is an attempt to reset channel.
        // We can know this by checking if new_token is a special value.
        let friend = self.get_friend(&friend_public_key).unwrap();
        let reset_token = friend.directional.calc_channel_reset_token();
        let balance_for_reset = friend.directional.balance_for_reset();

        if new_token == reset_token {
            // This is a reset message. We reset the token channel:
            let mut fself = await!(self.cancel_local_pending_requests(
                friend_public_key.clone()))?;

            let friend_mutation = FriendMutation::RemoteReset;
            let messenger_mutation = FunderMutation::FriendMutation((friend_public_key.clone(), friend_mutation));
            fself.apply_mutation(messenger_mutation);

            Ok(fself)
        } else {
            Ok(self)
        }
    }


    /// Reply to a request message with failure.
    #[async]
    fn reply_with_failure(self, 
                          remote_public_key: PublicKey,
                          request_send_funds: RequestSendFunds) -> Result<Self, HandleFriendError> {

        let pending_request = request_send_funds.create_pending_request();
        let (mut fself, failure_send_funds) = await!(self.create_failure_message(pending_request))?;

        let failure_op = FriendTcOp::FailureSendFunds(failure_send_funds);
        let friend_mutation = FriendMutation::PushBackPendingOperation(failure_op);
        let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
        fself.apply_mutation(messenger_mutation);

        Ok(fself)
    }

    /// Forward a request message to the relevant friend and token channel.
    fn forward_request(&mut self, mut request_send_funds: RequestSendFunds) {
        let index = request_send_funds.route.pk_index(self.state.get_local_public_key())
            .expect("We are not present in the route!");
        let prev_index = index.checked_sub(1).expect("We are the originator of this request");
        let next_index = index.checked_add(1).expect("Index out of range");
        
        let prev_pk = request_send_funds.route.pk_by_index(prev_index)
            .expect("Could not obtain previous public key");
        let next_pk = request_send_funds.route.pk_by_index(prev_index)
            .expect("Could not obtain next public key");

        let prev_friend = self.state.get_friends().get(&prev_pk)
            .expect("Previous friend not present");
        let next_friend = self.state.get_friends().get(&next_pk)
            .expect("Next friend not present");


        let total_trust = self.state.get_total_trust();
        let prev_trust = prev_friend.get_trust();
        let forward_trust = next_friend.get_trust();

        let two_pow_128 = BigUint::new(vec![0x1, 0x0u32, 0x0u32, 0x0u32, 0x0u32]);
        let numerator = (two_pow_128 * forward_trust) / (total_trust - &prev_trust);
        let usable_ratio = match numerator.to_u128() {
            Some(num) => Ratio::Numerator(num),
            None => Ratio::One,
        };

        let shared_credits = prev_trust.to_u128().unwrap_or(u128::max_value());

        // Add our freeze link
        request_send_funds.freeze_links.push(FunderFreezeLink {
            shared_credits,
            usable_ratio,
        });

        // Queue message to the relevant friend. Later this message will be queued to a specific
        // available token channel:
        let friend_mutation = FriendMutation::PushBackPendingRequest(request_send_funds.clone());
        let messenger_mutation = FunderMutation::FriendMutation((next_pk.clone(), friend_mutation));
        self.apply_mutation(messenger_mutation);
    }

    #[async]
    fn handle_request_send_funds(mut self, 
                               remote_public_key: PublicKey,
                               channel_index: u16,
                               request_send_funds: RequestSendFunds) -> Result<Self, HandleFriendError> {

        self.cache.freeze_guard.add_frozen_credit(&request_send_funds.create_pending_request());
        // TODO: Add rest of add/sub_frozen_credit

        // Find ourselves on the route. If we are not there, abort.
        let pk_pair = request_send_funds.route.find_pk_pair(
            &remote_public_key, 
            self.state.get_local_public_key())
            .expect("Could not find pair in request_send_funds route!");

        let index = match pk_pair {
            PkPairPosition::Dest => {
                self.punt_request_to_crypter(request_send_funds);
                return Ok(self);
            }
            PkPairPosition::NotDest(i) => {
                i.checked_add(1).expect("Route too long!")
            }
        };



        // The node on the route has to be one of our friends:
        let next_index = index.checked_add(1).expect("Route too long!");
        let next_public_key = request_send_funds.route.pk_by_index(next_index)
            .expect("index out of range!");
        let mut fself = if !self.state.get_friends().contains_key(next_public_key) {
            await!(self.reply_with_failure(remote_public_key.clone(), 
                                           channel_index,
                                           request_send_funds.clone()))?
        } else {
            self
        };


        // Perform DoS protection check:
        Ok(match fself.cache.freeze_guard.verify_freezing_links(&request_send_funds) {
            Some(()) => {
                // Add our freezing link, and queue message to the next node.
                fself.forward_request(request_send_funds);
                fself
            },
            None => {
                // Queue a failure message to this token channel:
                await!(fself.reply_with_failure(remote_public_key, 
                                               channel_index,
                                               request_send_funds))?
            },
        })
    }

    fn handle_response_send_funds(&mut self, 
                               remote_public_key: &PublicKey,
                               channel_index: u16,
                               response_send_funds: ResponseSendFunds,
                               pending_request: PendingFriendRequest) {

        self.cache.freeze_guard.sub_frozen_credit(&pending_request);
        match self.find_request_origin(&response_send_funds.request_id) {
            None => {
                // We are the origin of this request, and we got a response.
                // We should pass it back to crypter.
                self.messenger_tasks.push(
                    FunderTask::CrypterFunds(
                        CrypterFunds::ResponseReceived(ResponseReceived {
                            request_id: response_send_funds.request_id,
                            processing_fee_collected: response_send_funds.processing_fee_collected,
                            response_content: response_send_funds.response_content,
                        })
                    )
                );
            },
            Some((friend_public_key, channel_index)) => {
                // Queue this response message to another token channel:
                let response_op = FriendTcOp::ResponseSendFunds(response_send_funds);
                let slot_mutation = SlotMutation::PushBackPendingOperation(response_op);
                let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
                let messenger_mutation = FunderMutation::FriendMutation((friend_public_key, friend_mutation));
                self.apply_mutation(messenger_mutation);
            },
        }
    }

    #[async]
    fn handle_failure_send_funds(mut self, 
                               remote_public_key: &PublicKey,
                               channel_index: u16,
                               failure_send_funds: FailureSendFunds,
                               pending_request: PendingFriendRequest)
                                -> Result<Self, HandleFriendError> {

        self.cache.freeze_guard.sub_frozen_credit(&pending_request);
        let fself = match self.find_request_origin(&failure_send_funds.request_id) {
            None => {
                // We are the origin of this request, and we got a failure
                // We should pass it back to crypter.
                self.messenger_tasks.push(
                    FunderTask::CrypterFunds(
                        CrypterFunds::FailureReceived(FailureReceived {
                            request_id: failure_send_funds.request_id,
                            reporting_public_key: failure_send_funds.reporting_public_key,
                        })
                    )
                );
                self
            },
            Some((friend_public_key, channel_index)) => {
                let (mut fself, failure_send_funds) = await!(self.failure_message_add_signature(failure_send_funds, 
                                                               pending_request))?;
                // Queue this failure message to another token channel:
                let failure_op = FriendTcOp::FailureSendFunds(failure_send_funds);
                let slot_mutation = SlotMutation::PushBackPendingOperation(failure_op);
                let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
                let messenger_mutation = FunderMutation::FriendMutation((friend_public_key.clone(), friend_mutation));
                fself.apply_mutation(messenger_mutation);

                fself
            },
        };
        Ok(fself)
    }

    /// Process valid incoming operations from remote side.
    #[async]
    fn handle_move_token_output(mut self, 
                                remote_public_key: PublicKey,
                                channel_index: u16,
                                incoming_messages: Vec<IncomingFunds> )
                        -> Result<Self, HandleFriendError> {

        let mut fself = self;
        for incoming_message in incoming_messages {
            fself = match incoming_message {
                IncomingFunds::Request(request_send_funds) => 
                    await!(fself.handle_request_send_funds(remote_public_key.clone(), channel_index, 
                                                 request_send_funds))?,
                IncomingFunds::Response(IncomingResponseSendFunds {
                                                pending_request, incoming_response}) => {
                    fself.handle_response_send_funds(&remote_public_key, channel_index, 
                                                  incoming_response, pending_request);
                    fself
                },
                IncomingFunds::Failure(IncomingFailureSendFunds {
                                                pending_request, incoming_failure}) => {
                    await!(fself.handle_failure_send_funds(&remote_public_key, channel_index, 
                                                 incoming_failure, pending_request))?
                },
            }
        }
        Ok(fself)
    }

    /// Handle an error with incoming move token.
    fn handle_move_token_error(&mut self,
                               remote_public_key: &PublicKey,
                               channel_index: u16,
                               receive_move_token_error: ReceiveMoveTokenError) {
        // Send a message about inconsistency problem to AppManager:
        self.messenger_tasks.push(
            FunderTask::AppManagerFunds(
                AppManagerFunds::ReceiveMoveTokenError(receive_move_token_error)));


        // Clear current incoming inconsistency messages:
        let slot_mutation = SlotMutation::SetIncomingInconsistency(IncomingInconsistency::Empty);
        let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
        let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
        self.apply_mutation(messenger_mutation);


        let token_channel_slot = self.get_token_channel_slot(&remote_public_key, 
                                                              channel_index);
        // Send an InconsistencyError message to remote side:
        let current_token = token_channel_slot.directional
            .calc_channel_reset_token(channel_index);
        let balance_for_reset = token_channel_slot.directional
            .balance_for_reset();

        let inconsistency_error = FriendInconsistencyError {
            opt_ack: None,
            token_channel_index: channel_index,
            current_token,
            balance_for_reset,
        };

        self.messenger_tasks.push(
            FunderTask::FriendFunds(
                FriendFunds::InconsistencyError(inconsistency_error)));

        // Keep outgoing InconsistencyError message details in memory:
        let slot_mutation = SlotMutation::SetOutgoingInconsistency(OutgoingInconsistency::Sent);
        let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
        let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
        self.apply_mutation(messenger_mutation);
    }


    /// Queue as many messages as possible into available token channel.
    fn queue_outgoing_operations(&mut self,
                           remote_public_key: &PublicKey,
                           channel_index: u16,
                           out_tc: &mut OutgoingTokenChannel) -> Result<(), QueueOperationFailure> {

        let tc_slot = self.get_token_channel_slot(&remote_public_key, 
                                                    channel_index);

        // Set remote_max_debt if needed:
        let remote_max_debt = tc_slot
            .directional
            .remote_max_debt();

        if tc_slot.wanted_remote_max_debt != remote_max_debt {
            out_tc.queue_operation(FriendTcOp::SetRemoteMaxDebt(tc_slot.wanted_remote_max_debt))?;
        }

        // Set local_send_price if needed:
        let local_send_price = &tc_slot
            .directional
            .token_channel
            .state()
            .send_price
            .local_send_price;

        if tc_slot.wanted_local_send_price != *local_send_price {
            match &tc_slot.wanted_local_send_price {
                Some(wanted_local_send_price) => out_tc.queue_operation(FriendTcOp::EnableRequests(
                    wanted_local_send_price.clone()))?,
                None => out_tc.queue_operation(FriendTcOp::DisableRequests)?,
            };
        }

        // Send pending operations (responses and failures)
        // TODO: Possibly replace this clone with something more efficient later:
        let mut pending_operations = tc_slot.pending_operations.clone();
        while let Some(pending_operation) = pending_operations.pop_front() {
            out_tc.queue_operation(pending_operation)?;
            let slot_mutation = SlotMutation::PopFrontPendingOperation;
            let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
            let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
            self.apply_mutation(messenger_mutation);
        }

        // Send requests:
        let friend = self.state.get_friends().get(&remote_public_key).unwrap();

        let mut pending_requests = friend.pending_requests.clone();
        while let Some(pending_request) = pending_requests.pop_front() {
            out_tc.queue_operation(FriendTcOp::RequestSendFunds(pending_request))?;
            let friend_mutation = FriendMutation::PopFrontPendingRequest;
            let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
            self.apply_mutation(messenger_mutation);
        }

        Ok(())
    }

    /// Compose a large as possible message to send through the token channel to the remote side.
    /// The message should contain various operations, collected from:
    /// - Generic pending requests (Might be sent through any token channel).
    /// - Token channel specific pending responses/failures.
    /// - Commands that were initialized through AppManager.
    ///
    /// Any operations that will enter the message should be applied. For example, a failure
    /// message should cause the pending request to be removed.
    ///
    /// received_empty -- is the move token message we have just received empty?
    fn send_through_token_channel(&mut self, 
                                  remote_public_key: &PublicKey,
                                  channel_index: u16,
                                  received_empty: bool) {

        let token_channel_slot = self.get_token_channel_slot(remote_public_key, channel_index);
        let mut out_tc = token_channel_slot.directional
            .begin_outgoing_move_token(MAX_MOVE_TOKEN_LENGTH).unwrap();

        let res = self.queue_outgoing_operations(remote_public_key,
                                       channel_index,
                                       &mut out_tc);

        // If we had a real error, we should panic.
        // Otherwise
        match res {
            Ok(()) => {},
            Err(QueueOperationFailure {error, ..}) => {
                match error {
                    QueueOperationError::MaxLengthReached => {},
                    _ => unreachable!(),
                }
            }
        };

        let (operations, tc_mutations) = out_tc.done();

        // If there is nothing to send, and the transaction we have received is nonempty, send an empty message back as ack.
        //
        // If the message received is empty and there is nothing to send, we do nothing. (There
        // is no reason to send an ack for an empty message).

        // If we received an empty move token message, and we have nothing to send, 
        // we do nothing:
        if received_empty && operations.is_empty() {
            return;
        }

        for tc_mutation in tc_mutations {
            let directional_mutation = DirectionalMutation::TcMutation(tc_mutation);
            let slot_mutation = SlotMutation::DirectionalMutation(directional_mutation);
            let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
            let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
            self.apply_mutation(messenger_mutation);
        }
        let token_channel_slot = self.get_token_channel_slot(remote_public_key, channel_index);

        let rand_nonce = RandValue::new(&*self.rng);
        let friend_move_token_inner = FriendMoveTokenInner {
            operations,
            old_token: token_channel_slot.directional.new_token.clone(),
            rand_nonce,
        };

        let directional_mutation = DirectionalMutation::SetDirection(
            MoveTokenDirection::Outgoing(friend_move_token_inner));
        let slot_mutation = SlotMutation::DirectionalMutation(directional_mutation);
        let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
        let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
        self.apply_mutation(messenger_mutation);

        let token_channel_slot = self.get_token_channel_slot(remote_public_key, channel_index);
        let outgoing_move_token = token_channel_slot.directional.get_outgoing_move_token().unwrap();

        // Add a task for sending the outgoing move token:
        self.add_task(
            FunderTask::FriendFunds(
                FriendFunds::MoveToken(outgoing_move_token)));
    }


    /// Clear all pending inconsistency errors if exist
    fn clear_inconsistency_status(&mut self,
                               remote_public_key: &PublicKey,
                               channel_index: u16) {
        let tc_slot = self.get_token_channel_slot(&remote_public_key, channel_index);
        match tc_slot.inconsistency_status.incoming {
            IncomingInconsistency::Empty => {},
            _ => {
                let slot_mutation = SlotMutation::SetIncomingInconsistency(IncomingInconsistency::Empty);
                let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
                let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
                self.apply_mutation(messenger_mutation);
            },
        }

        let tc_slot = self.get_token_channel_slot(&remote_public_key, channel_index);
        match tc_slot.inconsistency_status.outgoing {
            OutgoingInconsistency::Empty => {},
            _ => {
                let slot_mutation = SlotMutation::SetOutgoingInconsistency(OutgoingInconsistency::Empty);
                let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
                let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
                self.apply_mutation(messenger_mutation);
            },
        }
    }

    /// Handle success with incoming move token.
    #[async]
    fn handle_move_token_success(mut self,
                               remote_public_key: PublicKey,
                               channel_index: u16,
                               receive_move_token_output: ReceiveMoveTokenOutput,
                               is_empty: bool) -> Result<Self, HandleFriendError> {

        self.clear_inconsistency_status(&remote_public_key,
                                        channel_index);

        match receive_move_token_output {
            ReceiveMoveTokenOutput::Duplicate => Ok(self),
            ReceiveMoveTokenOutput::RetransmitOutgoing(outgoing_move_token) => {
                // Retransmit last sent token channel message:
                self.messenger_tasks.push(
                    FunderTask::FriendFunds(
                        FriendFunds::MoveToken(outgoing_move_token)));
                Ok(self)
            },
            ReceiveMoveTokenOutput::Received(move_token_received) => {

                let MoveTokenReceived {incoming_messages, mutations} = 
                    move_token_received;

                // Apply all mutations:
                for directional_mutation in mutations {
                    let slot_mutation = SlotMutation::DirectionalMutation(directional_mutation);
                    let friend_mutation = FriendMutation::SlotMutation((channel_index, slot_mutation));
                    let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
                    self.apply_mutation(messenger_mutation);
                }


                let mut fself = await!(self.handle_move_token_output(remote_public_key.clone(),
                                               channel_index,
                                               incoming_messages))?;
                fself.send_through_token_channel(&remote_public_key,
                                                 channel_index,
                                                 is_empty);
                fself.initiate_load_funds(&remote_public_key,
                                          channel_index);
                Ok(fself)
            },
        }
    }


    #[async]
    fn handle_move_token(mut self, 
                         remote_public_key: PublicKey,
                         friend_move_token: FriendMoveToken) -> Result<Self,HandleFriendError> {

        // Find friend:
        let friend = match self.state.get_friends().get(&remote_public_key) {
            Some(friend) => friend,
            None => return Ok(self),
        };

        let channel_index = friend_move_token.token_channel_index;
        if channel_index >= friend.local_max_channels {
            // Tell remote side that we don't support such a high token channel index:
            self.messenger_tasks.push(
                FunderTask::FriendFunds(
                    FriendFunds::SetMaxTokenChannels(
                        FriendSetMaxTokenChannels {
                            max_token_channels: friend.local_max_channels,
                        }
                    )
                )
            );
            return Ok(self)
        }

        let token_channel_slot = self.get_token_channel_slot(&remote_public_key, 
                                                             channel_index);

        // Check if the channel is inconsistent.
        // This means that the remote side has sent an InconsistencyError message in the past.
        // In this case, we are not willing to accept new messages from the remote side until the
        // inconsistency is resolved.
        // TODO: Is this the correct behaviour?
        /*
        if let TokenChannelStatus::Inconsistent { .. } 
                    = token_channel_slot.tc_status {
            return Ok(self);
        };
        */


        let mut fself = await!(self.check_reset_channel(remote_public_key.clone(), 
                                           channel_index, 
                                           friend_move_token.new_token.clone()))?;

        let token_channel_slot = fself.get_token_channel_slot(&remote_public_key, 
                                                             channel_index);


        let is_empty = friend_move_token.operations.is_empty();

        // TODO: Possibly refactor this part into a function?
        let friend_move_token_inner = FriendMoveTokenInner {
            operations: friend_move_token.operations,
            old_token: friend_move_token.old_token,
            rand_nonce: friend_move_token.rand_nonce,
        };
        let receive_move_token_res = token_channel_slot.directional.simulate_receive_move_token(
            friend_move_token_inner,
            friend_move_token.new_token);

        Ok(match receive_move_token_res {
            Ok(receive_move_token_output) => {
                await!(fself.handle_move_token_success(remote_public_key.clone(),
                                             channel_index,
                                             receive_move_token_output,
                                             is_empty))?
            },
            Err(receive_move_token_error) => {
                fself.handle_move_token_error(&remote_public_key,
                                             channel_index,
                                             receive_move_token_error);
                fself
            },
        })
    }

    fn handle_inconsistency_error(&mut self, 
                                  remote_public_key: &PublicKey,
                                  friend_inconsistency_error: FriendInconsistencyError) {
        
        // Save incoming inconsistency details:
        let token_channel_index = friend_inconsistency_error.token_channel_index;
        let incoming = IncomingInconsistency::Incoming(ResetTerms {
            current_token: friend_inconsistency_error.current_token.clone(),
            balance_for_reset: friend_inconsistency_error.balance_for_reset,
        });

        let friend_mutation = FriendMutation::SetIncomingInconsistency(incoming);
        let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
        self.apply_mutation(messenger_mutation);

        // Obtain information about our reset terms:
        let tc_slot = self.get_token_channel_slot(
            remote_public_key, token_channel_index);
        let directional = &tc_slot.directional;
        let reset_token = directional.calc_channel_reset_token(token_channel_index);
        let balance_for_reset = directional.balance_for_reset();


        // Check if we should send an outgoing inconsistency message:
        let should_send_outgoing = match tc_slot.inconsistency_status.outgoing {
            OutgoingInconsistency::Empty => {
                let friend_mutation = FriendMutation::SetOutgoingInconsistency(OutgoingInconsistency::Sent);
                let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
                self.apply_mutation(messenger_mutation);
                true
            },
            OutgoingInconsistency::Sent => {
                let is_ack_valid = match friend_inconsistency_error.opt_ack {
                    Some(acked_reset_token) => acked_reset_token == reset_token,
                    None => false,
                };
                if is_ack_valid {
                    let friend_mutation = FriendMutation::SetOutgoingInconsistency(OutgoingInconsistency::Acked);
                    let messenger_mutation = FunderMutation::FriendMutation((remote_public_key.clone(), friend_mutation));
                    self.apply_mutation(messenger_mutation);
                    false
                } else {
                    true
                }
                
            },
            OutgoingInconsistency::Acked => false,
        };

        // Send an outgoing inconsistency message if required:
        if should_send_outgoing {
            let inconsistency_error = FriendInconsistencyError {
                opt_ack: Some(friend_inconsistency_error.current_token.clone()),
                token_channel_index,
                current_token: reset_token.clone(),
                balance_for_reset,
            };

            self.add_task(
                FunderTask::FriendFunds(
                    FriendFunds::InconsistencyError(inconsistency_error)));
        }

    }

    #[async]
    pub fn handle_friend_message(mut self, 
                                   remote_public_key: PublicKey, 
                                   friend_message: IncomingFriendFunds)
                                        -> Result<Self, HandleFriendError> {
        match friend_message {
            IncomingFriendFunds::MoveToken(friend_move_token) =>
                await!(self.handle_move_token(remote_public_key, friend_move_token)),
            IncomingFriendFunds::InconsistencyError(friend_inconsistency_error) => {
                self.handle_inconsistency_error(&remote_public_key, friend_inconsistency_error);
                Ok(self)
            }
        }
    }

}

