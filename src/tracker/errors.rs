error_chain! {
    errors {
        InvalidRequest(r: String) {
            description("invalid tracker request")
            display("invalid tracker request: {}", r)
        }

        InvalidResponse(r: &'static str) {
            description("invalid tracker response")
            display("invalid tracker response: {}", r)
        }

        TrackerError(e: String) {
            description("tracker error response")
            display("tracker error: {}", e)
        }

        EOF {
            description("the tracker closed the connection unexpectedly")
            display("tracker EOF")
        }

        IO {
            description("the tracker connection experienced an IO error")
            display("tracker IO error")
        }

        Timeout {
            description("the tracker failed to respond to the request in a timely manner")
            display("tracker timeout")
        }
    }
}

