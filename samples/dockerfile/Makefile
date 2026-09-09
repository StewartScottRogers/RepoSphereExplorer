IMAGE ?= repo-sphere/app
TAG   ?= local

.PHONY: build run lint scan clean

build:
	docker build --target runtime -t $(IMAGE):$(TAG) .

run: build
	docker compose up --build

lint:
	docker run --rm -i hadolint/hadolint < Dockerfile

scan: build
	docker scout cves $(IMAGE):$(TAG)

clean:
	docker compose down --volumes --remove-orphans
