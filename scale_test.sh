#!/bin/bash
echo "Testing Wasm Scalability..."
time (for i in {1..20}; do wasmedge my-app-aot.wasm > /dev/null & done; wait)

echo "--------------------------"

echo "Testing Docker Scalability..."
time (for i in {1..20}; do docker run --rm my-docker-app:latest > /dev/null & done; wait)