use super::{InstrumentOrderBook, OrderBookUpdater};
use crate::{
    error::DataError,
    event::{Market, MarketIter},
    exchange::Connector,
    subscription::{book::OrderBook, Map, SubKind},
    transformer::ExchangeTransformer,
    Identifier,
};
use async_trait::async_trait;
use barter_integration::{
    model::{Instrument, SubscriptionId},
    protocol::websocket::WsMessage,
    Transformer,
};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use tokio::sync::mpsc;

/// Todo:
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct MultiBookTransformer<Exchange, Kind, Updater> {
    pub book_map: Map<InstrumentOrderBook<Updater>>,
    phantom: PhantomData<(Exchange, Kind)>,
}

#[async_trait]
impl<Exchange, Kind, Updater> ExchangeTransformer<Exchange, Kind>
    for MultiBookTransformer<Exchange, Kind, Updater>
where
    Exchange: Connector + Send,
    Kind: SubKind<Event = OrderBook> + Send,
    Updater: OrderBookUpdater<OrderBook = Kind::Event> + Send,
    Updater::Update: Identifier<Option<SubscriptionId>> + for<'de> Deserialize<'de>,
{
    async fn new(
        ws_sink_tx: mpsc::UnboundedSender<WsMessage>,
        map: Map<Instrument>,
    ) -> Result<Self, DataError> {
        // Initialise InstrumentOrderBooks for all Subscriptions
        let (sub_ids, init_book_requests): (Vec<_>, Vec<_>) = map
            .0
            .into_iter()
            .map(|(sub_id, instrument)| {
                (
                    sub_id,
                    Updater::init::<Exchange, Kind>(ws_sink_tx.clone(), instrument),
                )
            })
            .unzip();

        // Await all initial OrderBook snapshot requests
        let init_order_books = futures::future::join_all(init_book_requests)
            .await
            .into_iter()
            .collect::<Result<Vec<InstrumentOrderBook<Updater>>, DataError>>()?;

        // Construct OrderBookMap if all requests successful
        let book_map = sub_ids
            .into_iter()
            .zip(init_order_books.into_iter())
            .collect::<Map<InstrumentOrderBook<Updater>>>();

        Ok(Self {
            book_map,
            phantom: PhantomData::default(),
        })
    }
}

impl<Exchange, Kind, Updater> Transformer for MultiBookTransformer<Exchange, Kind, Updater>
where
    Exchange: Connector,
    Kind: SubKind<Event = OrderBook>,
    Updater: OrderBookUpdater<OrderBook = Kind::Event>,
    Updater::Update: Identifier<Option<SubscriptionId>> + for<'de> Deserialize<'de>,
{
    type Error = DataError;
    type Input = Updater::Update;
    type Output = Market<Kind::Event>;
    type OutputIter = Vec<Result<Self::Output, Self::Error>>;

    fn transform(&mut self, update: Self::Input) -> Self::OutputIter {
        // Determine if the update has an identifiable SubscriptionId
        let subscription_id = match update.id() {
            Some(subscription_id) => subscription_id,
            None => return vec![],
        };

        // Retrieve the InstrumentOrderBook associated with this update (snapshot or delta)
        let book = match self.book_map.find_mut(&subscription_id) {
            Ok(book) => book,
            Err(unidentifiable) => return vec![Err(DataError::Socket(unidentifiable))],
        };

        // De-structure for ease
        let InstrumentOrderBook {
            instrument,
            book,
            updater,
        } = book;

        // Apply update (snapshot or delta) to OrderBook & generate Market<OrderBook> snapshot
        match updater.update(book, update) {
            Ok(Some(book)) => {
                MarketIter::<OrderBook>::from((Exchange::ID, instrument.clone(), book)).0
            }
            Ok(None) => vec![],
            Err(error) => vec![Err(error)],
        }
    }
}
