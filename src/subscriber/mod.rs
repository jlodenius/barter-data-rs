use self::{
    subscription::{SubKind, SubscriptionMeta, Subscription, SubscriptionMap},
    mapper::{SubscriptionMapper, WebSocketSubMapper},
    validator::SubscriptionValidator,
};
use crate::{
    exchange::Connector,
    Identifier,
};
use barter_integration::{error::SocketError, protocol::websocket::{WebSocket, connect}};
use async_trait::async_trait;
use std::marker::PhantomData;
use futures::SinkExt;

/// Todo:
pub mod subscription;
pub mod mapper;
pub mod validator;

/// Todo:
#[async_trait]
pub trait Subscriber<Validator>
where
    Validator: SubscriptionValidator,
{
    type SubMapper: SubscriptionMapper;

    async fn subscribe<Exchange, Kind>(
        subscriptions: &[Subscription<Exchange, Kind>],
    ) -> Result<(WebSocket, SubscriptionMap<Exchange, Kind>), SocketError>
    where
        Exchange: Connector + Send + Sync,
        Kind: SubKind + Send + Sync,
        Subscription<Exchange, Kind>: Identifier<Exchange::Channel> + Identifier<Exchange::Market>,
        Validator: 'async_trait;
}

pub struct WebSocketSubscriber<Validator> {
    phantom: PhantomData<Validator>,
}

#[async_trait]
impl<Validator> Subscriber<Validator> for WebSocketSubscriber<Validator>
where
    Validator: SubscriptionValidator,
{
    type SubMapper = WebSocketSubMapper;

    async fn subscribe<Exchange, Kind>(
        subscriptions: &[Subscription<Exchange, Kind>],
    ) -> Result<(WebSocket, SubscriptionMap<Exchange, Kind>), SocketError>
    where
        Exchange: Connector + Send + Sync,
        Kind: SubKind + Send + Sync,
        Subscription<Exchange, Kind>: Identifier<Exchange::Channel> + Identifier<Exchange::Market>,
        Validator: 'async_trait,
    {
        // Connect to exchange
        let mut websocket = connect(Exchange::base_url()).await?;

        // Map &[Subscription<Exchange, Kind>] to SubscriptionMeta
        let SubscriptionMeta {
            map,
            subscriptions,
            expected_responses,
        } = Self::SubMapper::map::<Exchange, Kind>(subscriptions);

        // Send Subscriptions
        for subscription in subscriptions {
            websocket.send(subscription).await?;
        }

        // Validate Subscriptions
        let map = Validator::validate::<Exchange, Kind>(
            map,
            &mut websocket,
            expected_responses
        ).await?;

        Ok((websocket, map))
    }
}
