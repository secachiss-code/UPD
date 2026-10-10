//! Привилегированный контроллер: кадр, личность peer, транзакции и жизненный цикл worker.

pub mod actions;
pub mod app_ops;
pub mod client;
pub mod codec;
pub mod core_ops;
pub mod dispatch;
pub mod drop;
pub mod harden;
pub mod journal;
pub mod net_ops;
pub mod owner;
pub mod peer;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod txn;
