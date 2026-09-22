.PHONY: help
help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

.PHONY: build
build: ## Build the binary
	cargo build

.PHONY: run
build: ## Run the Agent framework
	cargo run

.PHONY: inspector
build: ## Run the MCP inspector
	npx @modelcontextprotocol/inspector


lance-data-viewer:
	docker run --rm -p 8086:8080 \
    	-v ./data.db:/data:ro \
    	ghcr.io/lance-format/lance-data-viewer:lancedb-0.36.0

