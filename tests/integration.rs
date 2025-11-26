use firmware_controller::controller;
use futures::StreamExt;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum State {
    Idle,
    Active,
    Error,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Mode {
    Normal,
    Debug,
}

#[derive(Debug, PartialEq)]
pub enum TestError {
    InvalidState,
    OperationFailed,
}

#[controller]
mod test_controller {
    use super::*;

    pub struct Controller {
        #[controller(publish, getter = "get_current_state", setter = "change_state")]
        state: State,
        #[controller(publish(pub_setter), getter)]
        mode: Mode,
        #[controller(setter)]
        counter: u32,
    }

    impl Controller {
        #[controller(signal)]
        pub async fn error_occurred(&self, code: u32, message: heapless::String<32>);

        #[controller(signal)]
        pub async fn operation_complete(&self);

        pub async fn increment(&mut self) -> u32 {
            self.counter += 1;
            self.counter
        }

        pub async fn get_counter(&self) -> u32 {
            self.counter
        }

        pub async fn activate(&mut self) -> Result<(), TestError> {
            if self.state != State::Idle {
                return Err(TestError::InvalidState);
            }
            self.set_state(State::Active).await;
            self.operation_complete().await;
            Ok(())
        }

        pub async fn trigger_error(&mut self) -> Result<(), TestError> {
            self.set_state(State::Error).await;
            self.error_occurred(42, "Test error".try_into().unwrap())
                .await;
            Err(TestError::OperationFailed)
        }

        pub async fn return_nothing(&self) {}
    }
}

use test_controller::*;

#[test]
fn test_controller_basic_functionality() {
    // Create the controller before spawning the thread to avoid any race conditions.
    // The channels used for communication will buffer requests, so it's safe for the
    // client to start making calls even if the controller task hasn't fully started yet.
    let controller = Controller::new(State::Idle, Mode::Normal, 0);

    // Run the controller in a background thread.
    std::thread::spawn(move || {
        let executor = Box::leak(Box::new(embassy_executor::Executor::new()));
        executor.run(move |spawner| {
            spawner.spawn(controller_task(controller)).unwrap();
        });
    });

    // Run the test logic.
    futures::executor::block_on(async {
        // Create client.
        let mut client = ControllerClient::new();

        // Test 1: Subscribe to state changes.
        let mut state_stream = client.receive_state_changed().expect("Failed to subscribe");

        // Test 1a: First poll returns the initial (current) value.
        let initial_state = state_stream
            .next()
            .await
            .expect("Should receive initial state");
        assert_eq!(initial_state, State::Idle, "Initial state should be Idle");

        // Test 2: Subscribe to signals.
        let mut error_stream = client
            .receive_error_occurred()
            .expect("Failed to subscribe to error");
        let mut complete_stream = client
            .receive_operation_complete()
            .expect("Failed to subscribe to complete");

        // Test 3: Call a method and verify return value.
        let counter = client.get_counter().await;
        assert_eq!(counter, 0, "Initial counter should be 0");

        // Test 4: Call increment and verify it increases.
        let counter = client.increment().await;
        assert_eq!(counter, 1, "Counter should be 1 after increment");

        let counter = client.increment().await;
        assert_eq!(counter, 2, "Counter should be 2 after second increment");

        // Test 5: Call method that changes state and emits signal.
        let activate_result = client.activate().await;
        assert!(
            activate_result.is_ok(),
            "Activate should succeed from Idle state"
        );

        // Verify we received the state change (raw value, not Changed struct).
        let new_state = state_stream
            .next()
            .await
            .expect("Should receive state change");
        assert_eq!(new_state, State::Active, "New state should be Active");

        // Verify we received the operation_complete signal.
        let _complete = complete_stream
            .next()
            .await
            .expect("Should receive operation complete signal");

        // Test 6: Call method that returns error.
        let error_result = client.trigger_error().await;
        assert!(
            error_result.is_err(),
            "trigger_error should return an error"
        );
        assert_eq!(
            error_result.unwrap_err(),
            TestError::OperationFailed,
            "Should return OperationFailed error"
        );

        // Verify state changed to Error.
        let new_state = state_stream
            .next()
            .await
            .expect("Should receive state change");
        assert_eq!(new_state, State::Error, "New state should be Error");

        // Verify we received the error signal.
        let error_signal = error_stream
            .next()
            .await
            .expect("Should receive error signal");
        assert_eq!(error_signal.code, 42, "Error code should be 42");
        assert_eq!(
            error_signal.message.as_str(),
            "Test error",
            "Error message should match"
        );

        // Test 7: Try to activate again (should fail due to invalid state).
        let activate_result = client.activate().await;
        assert!(
            activate_result.is_err(),
            "Activate should fail from Error state"
        );
        assert_eq!(
            activate_result.unwrap_err(),
            TestError::InvalidState,
            "Should return InvalidState error"
        );

        // Test 8: Use pub_setter to change mode (backwards compatibility).
        client.set_mode(Mode::Debug).await;

        // Test 9: Call method with no return value.
        client.return_nothing().await;

        // Test 10: Use getter with custom name to get state.
        let state = client.get_current_state().await;
        assert_eq!(state, State::Error, "State should be Error");

        // Test 11: Use getter with default field name to get mode.
        let mode = client.mode().await;
        assert_eq!(mode, Mode::Debug, "Mode should be Debug");

        // Test 12: Use setter with custom name (new syntax).
        client.change_state(State::Idle).await;
        let state = client.get_current_state().await;
        assert_eq!(
            state,
            State::Idle,
            "State should be Idle after change_state"
        );

        // Test 13: Use setter without publish (independent setter).
        client.set_counter(100).await;
        let counter = client.get_counter().await;
        assert_eq!(counter, 100, "Counter should be 100 after set_counter");

        // If we get here, all tests passed.
    });
}

#[embassy_executor::task]
async fn controller_task(controller: Controller) {
    controller.run().await;
}
