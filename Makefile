NAME=synapse
VERSION=$(shell git rev-parse HEAD)
SEMVER_VERSION=$(shell grep version Cargo.toml | awk -F"\"" '{print $$2}' | head -n 1)
REPO=luminarys

compile:
	docker run --rm \
		-v cargo-cache:/root/.cargo \
		-v $$PWD:/volume \
		-w /volume \
		-it clux/muslrust:stable \
		cargo build --release --all
	sudo chown $$USER:$$USER -R target
	strip target/x86_64-unknown-linux-musl/release/synapse
	strip target/x86_64-unknown-linux-musl/release/sycli

build:
	docker build -t $(REPO)/$(NAME):$(VERSION) . -f Dockerfile.musl

run:
	docker run -v ~/Downloads/synapse:/synapse/downloads -t $(REPO)/$(NAME):$(VERSION)

tag-latest: build
	docker tag $(REPO)/$(NAME):$(VERSION) $(REPO)/$(NAME):latest
	docker push $(REPO)/$(NAME):latest

tag-semver: build
	if curl -sSL https://registry.hub.docker.com/v1/repositories/$(REPO)/$(NAME)/tags | jq -r ".[].name" | grep -q $(SEMVER_VERSION); then \
		echo "Tag $(SEMVER_VERSION) already exists" && exit 1 ;\
	fi
	docker tag $(REPO)/$(NAME):$(VERSION) $(REPO)/$(NAME):$(SEMVER_VERSION)
	docker push $(REPO)/$(NAME):$(SEMVER_VERSION)
