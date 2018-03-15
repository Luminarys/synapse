FROM alpine:latest

RUN apk add --no-cache ca-certificates
RUN adduser synapse -Du 1000 -h /synapse

ADD target/x86_64-unknown-linux-musl/release/synapse /usr/bin
ADD target/x86_64-unknown-linux-musl/release/sycli /usr/bin

EXPOSE 16493 \
       8412 \
       16362 \
       16309

USER synapse
RUN mkdir -p ~/.config
ADD example_config.toml /synapse/.config/synapse.toml
RUN sed -i "s/directory \= \".\/\"/directory \= \"\/synapse\/downloads\"/" ~/.config/synapse.toml && \
    sed -i "s/local \= true/local \= false/" ~/.config/synapse.toml

VOLUME /synapse/downloads

CMD ["synapse"]
