.PHONY: build dev

docker_tag := synapse

dev:
	docker build -t $(docker_tag)-dev -f Dockerfile.dev .

build:
	docker build -t $(docker_tag) .
