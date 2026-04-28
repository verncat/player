use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::trace;

pub mod peer_actor;
pub mod peer_registry;
pub mod server_actor;

#[derive(Debug, Clone)]
pub enum ConnectionState {
    Disconnected,
    Connecting { since: Instant },
    Connected,
}
/// Core actor trait - each actor processes messages
pub trait Actor: Send + 'static {
    type Message: Send + 'static;

    /// Handle a single message
    fn handle(&mut self, msg: Self::Message);

    /// Called when actor starts (optional hook)
    fn on_start(&mut self) {}

    /// Called when actor stops (optional hook)
    fn on_stop(&mut self) {}

    /// Optional periodic tick for background work
    fn tick(&mut self) {}
}

pub struct ActorHandle<M: Send> {
    pub(crate) sender: mpsc::UnboundedSender<ActorMessage<M>>,
}

impl<M: Send> Clone for ActorHandle<M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

impl<M: Send> ActorHandle<M> {
    pub fn send(&self, msg: M) -> Result<(), String> {
        self.sender
            .send(ActorMessage::UserMessage(msg))
            .map_err(|e| format!("Failed to send message: {}", e))
    }

    /// Request actor to stop gracefully
    pub fn stop(&self) -> Result<(), String> {
        self.sender
            .send(ActorMessage::Stop)
            .map_err(|e| format!("Failed to send stop signal: {}", e))
    }
}

/// Internal actor message wrapper
pub(crate) enum ActorMessage<M> {
    UserMessage(M),
    Stop,
    #[allow(dead_code)]
    Tick,
}

/// Actor system that manages actor lifecycle
pub struct ActorSystem {
    cancellation_token: CancellationToken,
}

impl ActorSystem {
    pub fn new() -> Self {
        ActorSystem {
            cancellation_token: CancellationToken::new(),
        }
    }

    /// Get a reference to the cancellation token for external tasks (e.g., listener)
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    /// Shutdown all spawned actors by cancelling the token
    pub fn shutdown(&self) {
        self.cancellation_token.cancel();
    }

    /// Spawn a new actor and return its handle
    pub fn spawn<A: Actor>(&self, mut actor: A) -> ActorHandle<A::Message> {
        let (sender, receiver) = mpsc::unbounded_channel::<ActorMessage<A::Message>>();
        let handle = ActorHandle {
            sender: sender.clone(),
        };

        let token = self.cancellation_token.child_token();

        // Start the actor event loop as a tokio task
        tokio::spawn(async move {
            actor.on_start();
            Self::run_actor_loop(&mut actor, receiver, token).await;
            actor.on_stop();
        });

        handle
    }

    /// Spawn a new actor with initialization callback and return its handle
    /// The callback receives the actor handle before on_start is called
    pub fn spawn_with_handle<A: Actor, F>(&self, mut actor: A, init: F) -> ActorHandle<A::Message>
    where
        F: FnOnce(&mut A, ActorHandle<A::Message>) + Send + 'static,
    {
        let (sender, receiver) = mpsc::unbounded_channel::<ActorMessage<A::Message>>();
        let handle = ActorHandle {
            sender: sender.clone(),
        };
        let handle_for_init = handle.clone();

        let token = self.cancellation_token.child_token();

        tokio::spawn(async move {
            init(&mut actor, handle_for_init);
            actor.on_start();
            Self::run_actor_loop(&mut actor, receiver, token).await;
            actor.on_stop();
        });

        handle
    }

    async fn run_actor_loop<A: Actor>(
        actor: &mut A,
        mut receiver: mpsc::UnboundedReceiver<ActorMessage<A::Message>>,
        cancellation_token: CancellationToken,
    ) {
        let tick_interval = Duration::from_millis(100);
        let mut _message_count = 0;
        let mut _tick_count = 0;

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    trace!(
                        "[actor_system] Cancellation token triggered, breaking loop"
                    );
                    break;
                }
                msg = receiver.recv() => {
                    match msg {
                        Some(ActorMessage::UserMessage(msg)) => {
                            _message_count += 1;
                            actor.handle(msg);
                        }
                        Some(ActorMessage::Stop) => {
                            trace!(
                                "[actor_system] Received Stop message, breaking loop"
                            );
                            break;
                        }
                        Some(ActorMessage::Tick) => {
                            _tick_count += 1;
                            trace!(
                                "[actor_system] Received explicit Tick message #{}",
                                _tick_count
                            );
                            actor.tick();
                        }
                        None => {
                            trace!(
                                "[actor_system] Channel disconnected, breaking loop"
                            );
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(tick_interval) => {
                    _tick_count += 1;
                    actor.tick();
                }
            }
        }
        trace!(
            "[actor_system] run_actor_loop ENDED - processed {} messages, {} ticks",
            _message_count, _tick_count
        );
    }
}

impl Default for ActorSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CounterActor {
        count: Arc<AtomicUsize>,
    }

    impl Actor for CounterActor {
        type Message = usize;

        fn handle(&mut self, msg: Self::Message) {
            self.count.fetch_add(msg, Ordering::SeqCst);
        }

        fn on_start(&mut self) {
            println!("Counter actor started");
        }

        fn on_stop(&mut self) {
            println!("Counter actor stopped");
        }
    }

    #[tokio::test]
    async fn test_actor_system() {
        let system = Arc::new(ActorSystem::new());

        let count = Arc::new(AtomicUsize::new(0));
        let actor = CounterActor {
            count: count.clone(),
        };

        let handle = system.spawn(actor);

        // Send some messages
        handle.send(1).unwrap();
        handle.send(2).unwrap();
        handle.send(3).unwrap();

        // Give actor time to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(count.load(Ordering::SeqCst), 6);

        // Stop the actor
        handle.stop().unwrap();

        // Give actor time to process the stop message
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn test_actor_system_shutdown() {
        let system = Arc::new(ActorSystem::new());

        let count = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));

        let actor1 = CounterActorWithStop {
            count: count.clone(),
            stopped: stopped.clone(),
        };
        let actor2 = CounterActorWithStop {
            count: count.clone(),
            stopped: stopped.clone(),
        };

        let _handle1 = system.spawn(actor1);
        let _handle2 = system.spawn(actor2);

        _handle1.send(1).unwrap();
        _handle2.send(2).unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(count.load(Ordering::SeqCst), 3);

        // Shutdown should stop all actors
        system.shutdown();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(stopped.load(Ordering::SeqCst), 2);
    }

    struct CounterActorWithStop {
        count: Arc<AtomicUsize>,
        stopped: Arc<AtomicUsize>,
    }

    impl Actor for CounterActorWithStop {
        type Message = usize;

        fn handle(&mut self, msg: Self::Message) {
            self.count.fetch_add(msg, Ordering::SeqCst);
        }

        fn on_stop(&mut self) {
            self.stopped.fetch_add(1, Ordering::SeqCst);
        }
    }
}
