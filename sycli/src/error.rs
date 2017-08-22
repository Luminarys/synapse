error_chain! {
    errors {
        FileIO {
            description("Failed to perform file IO")
                display("Failed to perform file IO")
        }
        Serialization {
            description("Failed to serialize structure")
                display("Failed to serialize structure")
        }
        Deserialization {
            description("Failed to deserialize structure")
                display("Failed to deserialize structure")
        }
        Websocket {
            description("Failed to handle websocket client")
                display("Failed to handle websocket client")
        }
        HTTP {
            description("HTTP transfer failed")
                display("HTTP transfer failed")
        }
    }
}
