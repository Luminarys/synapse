FROM rust:latest as build

ARG SYNAPSE_VERSION=master
ARG SYNAPSE_GIT_SRC=https://github.com/Luminarys/synapse.git

WORKDIR /usr/src

RUN git clone $SYNAPSE_GIT_SRC . \
  && [ "$SYNAPSE_VERSION" != 'master' ] && git checkout tags/$SYNAPSE_VERSION || git checkout master \
  ;
RUN cargo build --release --all

FROM debian:latest

ENV SYNAPSE_HOME=/opt/synapse

RUN apt-get update \
  && apt-get -y install libssl1.1 \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --comment "Synapse" --home ${SYNAPSE_HOME} --shell /usr/sbin/nologin --system synapse \
  && mkdir -p ${SYNAPSE_HOME} \
  && chown synapse:synapse ${SYNAPSE_HOME} \
  ;

COPY --from=build /usr/src/target/release/synapse /usr/src/target/release/sycli /usr/local/bin/

USER synapse
WORKDIR $SYNAPSE_HOME
CMD ["synapse"]
