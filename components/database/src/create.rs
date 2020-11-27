use rusqlite::{self, params, Connection};

/// Create a new Offset database
fn create_database(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    // TODO: A better way to do this?
    tx.execute("PRAGMA foreign_keys = ON;", params![])?;

    tx.execute(
        "CREATE TABLE applications (
             app_public_key             BLOB NOT NULL PRIMARY KEY,
             app_name                   TEXT NOT NULL UNIQUE,
             last_online                INTEGER NOT NULL,
             is_enabled                 BOOL NOT NULL,

             perm_info_docs             BOOL NOT NULL,
             perm_info_friends          BOOL NOT NULL,
             perm_buy                   BOOL NOT NULL,
             perm_sell                  BOOL NOT NULL,
             perm_config_card           BOOL NOT NULL,
             perm_config_access         BOOL NOT NULL,

             FOREIGN KEY(app_public_key) 
                REFERENCES applications(app_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_applications ON applications(app_public_key, app_name);",
        params![],
    )?;

    // Single row is enforced in this table according to https://stackoverflow.com/a/33104119
    tx.execute(
        "CREATE TABLE node(
             id                       INTEGER PRIMARY KEY CHECK (id == 0), -- enforce single row
             version                  INTEGER NOT NULL,  -- Database version
             local_public_key         BLOB NOT NULL
            );",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE friends(
             friend_public_key          BLOB NOT NULL PRIMARY KEY,
             friend_name                TEXT NOT NULL UNIQUE,
             -- Friend's name
             is_enabled                 BOOL NOT NULL,
             -- Do we allow connectivity with this friend?
             is_consistent              BOOL NOT NULL
             -- Is the channel with this friend consistent?
            );",
        params![],
    )?;

    // friend public key index:
    tx.execute(
        "CREATE UNIQUE INDEX idx_friends_friend_public_key ON friends(friend_public_key);",
        params![],
    )?;

    // public name index:
    tx.execute(
        "CREATE UNIQUE INDEX idx_friends_name ON friends(friend_name);",
        params![],
    )?;

    // Relays list we have received from the friend
    // We use those relays to connect to the friend.
    tx.execute(
        "CREATE TABLE received_friend_relays(
             friend_public_key        BLOB NOT NULL,
             relay_public_key         BLOB NOT NULL,
             port                     BLOB NOT NULL,
             address                  TEXT NOT NULL,
             PRIMARY KEY(friend_public_key, relay_public_key),
             FOREIGN KEY(friend_public_key)
                REFERENCES friends(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_received_friend_relays ON received_friend_relays(friend_public_key, relay_public_key);",
        params![],
    )?;

    // Relays list we have sent to the friend
    // The friend will use those relays to connect to us
    tx.execute(
        "CREATE TABLE sent_friend_relays(
             friend_public_key        BLOB NOT NULL,
             relay_public_key         BLOB NOT NULL,
             port                     BLOB NOT NULL,
             address                  TEXT NOT NULL,
             is_remove                BOOL NOT NULL,
             -- Is marked for removal?
             generation               BLOB,
             -- At which generation this relay was added/removed?
             -- NULL if change was already acked.
             CHECK (NOT (is_remove == true AND generation == NULL)),
             PRIMARY KEY(friend_public_key, relay_public_key),
             FOREIGN KEY(friend_public_key)
                REFERENCES friends(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_sent_friend_relays ON sent_friend_relays(friend_public_key, relay_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE INDEX idx_sent_friend_relays_generation ON sent_friend_relays(generation);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE currency_configs(
             friend_public_key          BLOB NOT NULL,
             currency                   TEXT NOT NULL,
             rate                       BLOB NOT NULL,
             remote_max_debt            BLOB NOT NULL,
             is_open                    BOOL NOT NULL,
             PRIMARY KEY(friend_public_key, currency),
             FOREIGN KEY(friend_public_key) 
                REFERENCES friends(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_currency_configs ON currency_configs(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE consistent_channels(
             friend_public_key          BLOB NOT NULL PRIMARY KEY,
             is_consistent              BOOL NOT NULL 
                                        CHECK (is_consistent == true)
                                        DEFAULT true,
             move_token_counter         BLOB NOT NULL,
             is_incoming                BOOL NOT NULL,

             FOREIGN KEY(friend_public_key, is_consistent) 
                REFERENCES friends(friend_public_key, is_consistent)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_consistent_channels ON consistent_channels(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE consistent_channels_incoming(
             friend_public_key      BLOB NOT NULL PRIMARY KEY,
             is_incoming            BOOL NOT NULL 
                                    CHECK (is_incoming == true)
                                    DEFAULT true,
             old_token              BLOB NOT NULL,
             new_token              BLOB NOT NULL,

             FOREIGN KEY(friend_public_key, is_incoming) 
                REFERENCES consistent_channels(friend_public_key, is_incoming)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_consistent_channels_incoming ON consistent_channels_incoming(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE consistent_channels_outgoing(
             friend_public_key          BLOB NOT NULL PRIMARY KEY,
             is_incoming                BOOL NOT NULL 
                                        CHECK (is_incoming == false)
                                        DEFAULT false,
             move_token_out             BLOB NOT NULL,

             -- Do we save a previously received hashed incoming move token?
             is_with_incoming           BOOL NOT NULL,

             FOREIGN KEY(friend_public_key, is_incoming) 
                REFERENCES consistent_channels(friend_public_key, is_incoming)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_consistent_channels_outgoing ON consistent_channels_outgoing(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE consistent_channels_outgoing_with_incoming(
             friend_public_key          BLOB NOT NULL PRIMARY KEY,
             is_with_incoming           BOOL NOT NULL 
                                        CHECK (is_with_incoming == true)
                                        DEFAULT true,

             -- Data saved for last incoming move token.
             -- The list of balances for the last incoming move token is saved in a separate table.
             old_token                  BLOB NOT NULL,
             move_token_counter         BLOB NOT NULL,
             new_token                  BLOB NOT NULL,

             FOREIGN KEY(friend_public_key, is_with_incoming) 
                REFERENCES consistent_channels_outgoing(friend_public_key, is_with_incoming)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_consistent_channels_outgoing_with_incoming ON consistent_channels_outgoing_with_incoming(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE last_incoming_balances(
             friend_public_key          BLOB NOT NULL,
             currency                   TEXT NOT NULL,

             balance                    BLOB NOT NULL,
             local_pending_debt         BLOB NOT NULL,
             remote_pending_debt        BLOB NOT NULL,
             in_fees                    BLOB NOT NULL,
             out_fees                   BLOB NOT NULL,

             PRIMARY KEY(friend_public_key, currency),
             FOREIGN KEY(friend_public_key) 
                REFERENCES consistent_channels_outgoing_with_incoming(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_last_incoming_balances ON last_incoming_balances(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE inconsistent_channels(
             friend_public_key                      BLOB NOT NULL PRIMARY KEY,
             is_consistent                          BOOL NOT NULL 
                                                    CHECK (is_consistent == false)
                                                    DEFAULT false,
             -- Last seen incoming move token:
             opt_move_token_in                      BLOB,
             -- Local reset terms:
             local_reset_token                      BLOB NOT NULL,
             local_reset_move_token_counter         BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, is_consistent) 
                REFERENCES friends(friend_public_key, is_consistent)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_inconsistent_channels ON inconsistent_channels(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE local_reset_terms(
             friend_public_key              BLOB NOT NULL,
             currency                       TEXT NOT NULL,
             balance                        BLOB NOT NULL,
             in_fees                        BLOB NOT NULL,
             out_fees                       BLOB NOT NULL,
             PRIMARY KEY(friend_public_key, currency),
             FOREIGN KEY(friend_public_key) 
                REFERENCES inconsistent_channels(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX local_reset_terms_idx ON local_reset_terms(friend_public_key, currency);",
        params![],
    )?;

    // Inconsistent channels that contain remote reset terms.
    tx.execute(
        "CREATE TABLE inconsistent_channels_with_remote(
             friend_public_key                      BLOB NOT NULL PRIMARY KEY,
             -- Remote reset terms:
             remote_reset_token                     BLOB NOT NULL,
             remote_reset_move_token_counter        BLOB NOT NULL,
             FOREIGN KEY(friend_public_key) 
                REFERENCES inconsistent_channels(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX inconsistent_channels_with_remote_idx ON 
        inconsistent_channels_with_remote(friend_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE remote_reset_terms(
             friend_public_key              BLOB NOT NULL,
             currency                       TEXT NOT NULL,
             balance                        BLOB NOT NULL,
             in_fees                        BLOB NOT NULL,
             out_fees                       BLOB NOT NULL,
             PRIMARY KEY(friend_public_key, currency),
             FOREIGN KEY(friend_public_key) 
                REFERENCES inconsistent_channels_with_remote(friend_public_key)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX remote_reset_terms_idx ON remote_reset_terms(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE local_currencies(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             FOREIGN KEY(friend_public_key) 
                REFERENCES consistent_channels(friend_public_key)
                ON DELETE CASCADE,
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES currency_configs(friend_public_key, currency)
                ON DELETE CASCADE,
             PRIMARY KEY(friend_public_key, currency)
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_local_currencies ON local_currencies(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE remote_currencies(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             FOREIGN KEY(friend_public_key) 
                REFERENCES consistent_channels(friend_public_key)
                ON DELETE CASCADE,
             PRIMARY KEY(friend_public_key, currency)
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_remote_currencies ON remote_currencies(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE mutual_credits(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             balance                  BLOB NOT NULL,
             local_pending_debt       BLOB NOT NULL,
             remote_pending_debt      BLOB NOT NULL,
             in_fees                  BLOB NOT NULL,
             out_fees                 BLOB NOT NULL,
             PRIMARY KEY(friend_public_key, currency)
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES local_currencies(friend_public_key, currency)
                ON DELETE CASCADE
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES remote_currencies(friend_public_key, currency)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_mutual_credits ON mutual_credits(friend_public_key, currency);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE local_open_transactions(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             src_hashed_lock          BLOB NOT NULL,
             route                    BLOB NOT NULL,
             dest_payment             BLOB NOT NULL,
             total_dest_payment       BLOB NOT NULL,
             invoice_hash             BLOB NOT NULL,
             hmac                     BLOB NOT NULL,
             left_fees                BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES mutual_credits(friend_public_key, currency)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_local_open_transactions ON local_open_transactions(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE remote_open_transactions(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             src_hashed_lock          BLOB NOT NULL,
             route                    BLOB NOT NULL,
             dest_payment             BLOB NOT NULL,
             total_dest_payment       BLOB NOT NULL,
             invoice_hash             BLOB NOT NULL,
             hmac                     BLOB NOT NULL,
             left_fees                BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES mutual_credits(friend_public_key, currency)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_remote_open_transactions ON remote_open_transactions(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE pending_user_requests(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             generation               BLOB NOT NULL UNIQUE,
             src_hashed_lock          BLOB NOT NULL,
             route                    BLOB NOT NULL,
             dest_payment             BLOB NOT NULL,
             total_dest_payment       BLOB NOT NULL,
             invoice_hash             BLOB NOT NULL,
             hmac                     BLOB NOT NULL,
             left_fees                BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES mutual_credits(friend_public_key, currency)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_user_requests_request_id ON pending_user_requests(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_user_requests_generation ON pending_user_requests(generation);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE pending_requests(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             generation               BLOB NOT NULL UNIQUE,
             src_hashed_lock          BLOB NOT NULL,
             route                    BLOB NOT NULL,
             dest_payment             BLOB NOT NULL,
             total_dest_payment       BLOB NOT NULL,
             invoice_hash             BLOB NOT NULL,
             hmac                     BLOB NOT NULL,
             left_fees                BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES mutual_credits(friend_public_key, currency)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_requests_request_id ON pending_requests(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_requests_generation ON pending_requests(generation);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE pending_backwards(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             generation               BLOB NOT NULL UNIQUE,
             backwards_type           TEXT CHECK (backwards_type IN ('R', 'C')) NOT NULL,
             -- R: Response, C: Cancel
             FOREIGN KEY(friend_public_key, currency) 
                REFERENCES mutual_credits(friend_public_key, currency)
                ON DELETE CASCADE,
             FOREIGN KEY(request_id) 
                REFERENCES pending_requests(request_id)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_backwards_request_id ON pending_backwards(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_backwards_generation ON pending_backwards(generation);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE pending_backwards_responses(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             backwards_type           TEXT NOT NULL 
                                      CHECK (backwards_type == 'R')
                                      DEFAULT 'R',
             src_hashed_lock          BLOB NOT NULL,
             serial_num               BLOB NOT NULL,
             signature                BLOB NOT NULL,
             FOREIGN KEY(friend_public_key, currency, request_id, backwards_type) 
                REFERENCES pending_backwards(friend_public_key, currency, request_id, backwards_type)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_backwards_responses ON pending_backwards_responses(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE pending_backwards_cancels(
             friend_public_key        BLOB NOT NULL,
             currency                 TEXT NOT NULL,
             request_id               BLOB NOT NULL PRIMARY KEY,
             backwards_type           TEXT NOT NULL 
                                      CHECK (backwards_type == 'C')
                                      DEFAULT 'C',
             FOREIGN KEY(friend_public_key, currency, request_id, backwards_type) 
                REFERENCES pending_backwards(friend_public_key, currency, request_id, backwards_type)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_pending_backwards_cancels ON pending_backwards_cancels(request_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE relays(
             relay_public_key         BLOB NOT NULL PRIMARY KEY,
             address                  TEXT NOT NULL,
             relay_name               TEXT NOT NULL
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_relays ON relays(relay_public_key);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE index_servers(
             index_public_key         BLOB NOT NULL PRIMARY KEY,
             address                  TEXT NOT NULL,
             index_name               TEXT NOT NULL
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_index_servers ON index_servers(index_public_key);",
        params![],
    )?;

    // Ongoing payments that were not yet finalized
    // TODO: Complete open payments information:
    // - Pending requests/responses?
    // - Stage information? (See state.rs in funder)
    tx.execute(
        "CREATE TABLE open_payments(
            payment_id          BLOB NOT NULL PRIMARY KEY,
            dest_public_key     BLOB NOT NULL,
            currency            TEXT NOT NULL,
            amount              BLOB NOT NULL,
            description         TEXT NOT NULL,
            src_plain_lock      BLOB NOT NULL
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_open_payments ON open_payments(payment_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE open_invoices(
             invoice_id      BLOB NOT NULL PRIMARY KEY,
             psk             BLOB NOT NULL,
             description     TEXT NOT NULL,
             data_hash       BLOB NOT NULL,
             currency        TEXT NOT NULL,
             opt_amount      BLOB
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_open_invoices ON open_invoices(invoice_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE open_invoices_relays(
             invoice_id      BLOB NOT NULL,
             relay_address   TEXT NOT NULL,
             relay_port      BLOB NOT NULL,
             PRIMARY KEY(invoice_id, relay_address),
             FOREIGN KEY(invoice_id) 
                 REFERENCES open_invoices(invoice_id)
                 ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_open_invoices_relays ON open_invoices_relays(invoice_id, relay_address);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE invoice_actions(
             invoice_id             BLOB NOT NULL,
             action_id              BLOB NOT NULL,
             description            TEXT NOT NULL,
             invoice_hash           BLOB NOT NULL UNIQUE,
             -- invoiceHash = sha512/256(action_id || hash(description) || data_hash)
             opt_src_hashed_lock    BLOB,
             -- src_hashed_lock is provided by the buyer/payer during the commitment.
             PRIMARY KEY(invoice_id, action_id),
             FOREIGN KEY(invoice_id) 
                 REFERENCES open_invoices(invoice_id)
                 ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_invoice_actions ON invoice_actions(invoice_id, action_id);",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE invoice_actions_requests(
             invoice_id             BLOB NOT NULL,
             action_id              BLOB NOT NULL,
             request_id             BLOB NOT NULL PRIMARY KEY,
             FOREIGN KEY(request_id) 
                 REFERENCES remote_open_transactions(request_id)
                 ON DELETE RESTRICT,
             FOREIGN KEY(invoice_id, action_id) 
                 REFERENCES invoice_actions(invoice_id, action_id)
                 ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.execute(
        "CREATE INDEX idx_actions_requests ON invoice_actions_requests(invoice_id, action_id);",
        params![],
    )?;

    tx.execute(
        "CREATE UNIQUE INDEX idx_actions_requests_request_id ON invoice_actions_requests(request_id);",
        params![],
    )?;

    // Documents table, allowing total order on all items payments and invoices
    tx.execute(
        "CREATE TABLE events(
             counter      BLOB NOT NULL PRIMARY KEY,
             time         INTEGER NOT NULL,
             event_type   TEXT CHECK (event_type IN ('P', 'I', 'R')) NOT NULL,
             -- Event type: P: Payment, I: Invoice, R: Friend Removal
             -- Information about the application that trigerred this event:
             app_public_key             BLOB NOT NULL,
             app_name                   TEXT NOT NULL
            );",
        params![],
    )?;

    // Balances table, showing the exact balance for every event.
    // The numbers shown represent the numbers right after the event occurred.
    tx.execute(
        "CREATE TABLE event_balances(
              counter      BLOB NOT NULL, 
              currency     TEXT NOT NULL,
              amount       BLOB NOT NULL,
              in_fees      BLOB NOT NULL,
              out_fees     BLOB NOT NULL,
              event_type   TEXT CHECK (event_type IN ('P', 'I', 'F')) NOT NULL,
              -- Event type: P: Payment, I: Invoice, F: Friend inconsistency
 
              PRIMARY KEY(counter, currency)
              FOREIGN KEY(counter) 
                 REFERENCES events(counter)
                 ON DELETE CASCADE
             );",
        params![],
    )?;

    tx.execute(
        "CREATE TABLE payment_events(
             counter             BLOB NOT NULL PRIMARY KEY,
             event_type          TEXT CHECK (event_type == 'P') 
                                 DEFAULT 'P' 
                                 NOT NULL,
             response_hash       BLOB NOT NULL,
             dest_public_key     BLOB NOT NULL,
             serial_num          BLOB NOT NULL,
             action_id           BLOB NOT NULL,
             currency            TEXT NOT NULL,
             amount              BLOB NOT NULL,
             description         TEXT NOT NULL,
             data_hash           BLOB NOT NULL,
             signature           BLOB NOT NULL,
             UNIQUE (dest_public_key, serial_num),
             -- A node can not issue the same invoice sequence number twice
             FOREIGN KEY(counter, event_type) 
                REFERENCES events(counter, event_type)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    // TODO: Add relevant indexes?
    // - Search using description?
    // - Search using counter?
    // - How to make interface more flexible?
    //      Allowing user to make interesting queries?

    tx.execute(
        "CREATE TABLE invoice_events (
             counter         BLOB NOT NULL PRIMARY KEY,
             event_type      TEXT CHECK (event_type == 'I') 
                             DEFAULT 'I'
                             NOT NULL,
             serial_num      BLOB NOT NULL UNIQUE,
             currency        TEXT NOT NULL,
             amount          BLOB NOT NULL,
             description     TEXT NOT NULL,
             data_hash       BLOB NOT NULL,
             FOREIGN KEY(counter, event_type) 
                REFERENCES events(counter, event_type)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    // A case of friend inconsistency or friend removal
    tx.execute(
        "CREATE TABLE friend_inconsistency_events (
             counter                BLOB NOT NULL PRIMARY KEY,
             event_type             TEXT CHECK (event_type == 'F') 
                                    DEFAULT 'F'
                                    NOT NULL,
             friend_public_key      BLOB NOT NULL,
             friend_name            TEXT NOT NULL,
             FOREIGN KEY(counter, event_type) 
                REFERENCES events(counter, event_type)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    // Balances (per currency) in case of friend inconsistency event.
    // Friend inconsistency can be one of: 1. Channel reset, 2. Friend removal, 3. currency removal
    tx.execute(
        "CREATE TABLE friend_inconsistency_event_balances (
             counter                BLOB NOT NULL,
             currency               TEXT NOT NULL,
             old_balance            BLOB NOT NULL,
             old_in_fees            BLOB NOT NULL,
             old_out_fees           BLOB NOT NULL,
             new_balance            BLOB NOT NULL,
             new_in_fees            BLOB NOT NULL,
             new_out_fees           BLOB NOT NULL,
             PRIMARY KEY(counter, currency),
             FOREIGN KEY(counter) 
                REFERENCES friend_inconsistency_events(counter)
                ON DELETE CASCADE
            );",
        params![],
    )?;

    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_db() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_database(&mut conn).unwrap();
    }
}
