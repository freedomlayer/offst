use futures::{FutureExt, TryFutureExt, StreamExt, SinkExt};
use futures::channel::mpsc;
use futures::task::{Spawn, SpawnExt};

use proto::app_server::messages::{AppToAppServer, AppServerToApp,
    AppPermissions, NodeReport, NodeReportMutation};
use proto::funder::messages::ResponseReceived;
use proto::index_client::messages::ClientResponseRoutes;

use common::state_service::{state_service, StateClient};
use common::mutable_state::BatchMutable;
use common::multi_consumer::{multi_consumer_service, MultiConsumerClient};

use crate::connect::NodeConnectionTuple;

#[derive(Debug)]
pub enum NodeConnectionError {
    SpawnError,
}

#[derive(Clone)]
pub struct NodeConnection {
    sender: mpsc::Sender<AppToAppServer>,
    app_permissions: AppPermissions,
    report_client: StateClient<BatchMutable<NodeReport>,Vec<NodeReportMutation>>,
    routes_mc: MultiConsumerClient<ClientResponseRoutes>,
    send_funds_mc: MultiConsumerClient<ResponseReceived>,
}

// TODO:
pub struct AppReport;
pub struct AppConfig;
pub struct AppRoutes;
pub struct AppSendFunds;

impl NodeConnection {
    pub fn new<S>(conn_tuple: NodeConnectionTuple, spawner: &mut S) 
        -> Result<Self, NodeConnectionError> 
    where
        S: Spawn,
    {
        let (app_permissions, opt_node_report, (sender, mut receiver)) = conn_tuple;

        // Spawn report service:
        assert_eq!(app_permissions.reports, opt_node_report.is_some());
        let is_reports = opt_node_report.is_some();

        let (mut incoming_mutations_sender, incoming_mutations) = mpsc::channel(0);
        let (requests_sender, incoming_requests) = mpsc::channel(0);
        let report_client = StateClient::new(requests_sender);
        if let Some(node_report) = opt_node_report {
            let state_service_fut = state_service(incoming_requests,
                          BatchMutable(node_report),
                          incoming_mutations)
                .map_err(|e| error!("state_service() error: {:?}", e))
                .map(|_| ());
            spawner.spawn(state_service_fut)
                .map_err(|_| NodeConnectionError::SpawnError);
        }

        let (mut incoming_routes_sender, incoming_routes) = mpsc::channel(0);
        let (requests_sender, incoming_requests) = mpsc::channel(0);
        let routes_mc = MultiConsumerClient::new(requests_sender);
        let routes_fut = multi_consumer_service(incoming_routes, incoming_requests)
            .map_err(|e| error!("Routes multi_consumer_service() error: {:?}", e))
            .map(|_| ());
        spawner.spawn(routes_fut)
                .map_err(|_| NodeConnectionError::SpawnError);

        let (mut incoming_send_funds_sender, incoming_send_funds) = mpsc::channel(0);
        let (requests_sender, incoming_requests) = mpsc::channel(0);
        let send_funds_mc = MultiConsumerClient::new(requests_sender);
        let send_funds_fut = multi_consumer_service(incoming_send_funds, incoming_requests)
            .map_err(|e| error!("Routes multi_consumer_service() error: {:?}", e))
            .map(|_| ());
        spawner.spawn(send_funds_fut)
                .map_err(|_| NodeConnectionError::SpawnError);
        
        async move {
            while let Some(message) = await!(receiver.next()) {
                match message {
                    AppServerToApp::ResponseReceived(response_received) => {
                        let _ = await!(incoming_send_funds_sender.send(response_received));
                    },
                    AppServerToApp::Report(_node_report) => {
                        // TODO: Maybe somehow redesign the type AppServerToApp
                        // so that we don't have this edge case?
                        error!("Received unexpected AppServerToApp::Report message. Aborting.");
                        return;
                    },
                    AppServerToApp::ReportMutations(node_report_mutations) => {
                        if !is_reports {
                            error!("Received unexpected AppServerToApp::ReportMutations message. Aborting.");
                            return;
                        }
                        let _ = await!(incoming_mutations_sender.send(node_report_mutations));
                    },
                    AppServerToApp::ResponseRoutes(client_response_routes) => {
                        let _ = await!(incoming_routes_sender.send(client_response_routes));
                    },
                }
            }
        };

        Ok(NodeConnection {
            sender,
            app_permissions,
            report_client,
            routes_mc,
            send_funds_mc,
        })
    }

    pub fn report() -> Option<AppReport> {
        unimplemented!();
    }

    /*
    pub async fn report() -> Option<(NodeReport, mpsc::Receiver<NodeReportMutation>)> {
        unimplemented!();
    }
    */

    pub fn config() -> Option<AppConfig> {
        unimplemented!();
    }

    pub fn routes() -> Option<AppRoutes> {
        unimplemented!();
    }

    pub fn send_funds() -> Option<AppSendFunds> {
        unimplemented!();
    }
}
