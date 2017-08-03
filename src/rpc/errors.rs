error_chain! {
    errors {
        EOF {
            description("Unexpected socket EOF")
            display("Unexpected socket EOF")
        }

        Timeout {
            description("Connection timeout")
                display("Connection timeout")
        }

        BadPayload(s: &'static str) {
            description("Failed to decode payload")
                display("Bad payload: {}", s)
        }
    }
}

