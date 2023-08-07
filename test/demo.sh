#!/usr/bin/env bash
set -x

# Configuration:

modules=(
    "proxy_wasm_rust_rate_limiting"
)

DEMO_KONG_CONTAINER="${DEMO_KONG_CONTAINER:-kong-wasmx}"
DEMO_KONG_IMAGE="${DEMO_KONG_IMAGE:-kong/kong:3.2.0.0-ubuntu}"

################################################################################

if [[ "$1" == "stop" ]]
then
    docker stop $DEMO_KONG_CONTAINER
    docker rm $DEMO_KONG_CONTAINER
    exit 0
fi


(
    cd ..
    cargo build --target=wasm32-wasi --release || exit 1
) || exit 1

mkdir -p wasm

script_dir=$(dirname $(realpath $0))

for module in "${modules[@]}"
do
    cp ../target/wasm32-wasi/release/$module.wasm wasm/
done

docker stop $DEMO_KONG_CONTAINER
docker rm $DEMO_KONG_CONTAINER

wasm_modules=""
for module in "${modules[@]}"
do
    if [ "$wasm_modules" = "" ]
    then
        wasm_modules="/wasm/$module.wasm"
    else
        wasm_modules="$wasm_modules,/wasm/$module.wasm"
    fi
done

docker run -d --name "$DEMO_KONG_CONTAINER" \
    --add-host=host.docker.internal:$(ip -j address | jq -r '[ .[] | select(.ifname | test("^[ew]")) | .addr_info[] | select(.family == "inet") | .local ][0]') \
    -v "$script_dir/config:/kong/config/" \
    -v "$script_dir/wasm:/wasm" \
    -e "KONG_LOG_LEVEL=info" \
    -e "KONG_DATABASE=off" \
    -e "KONG_NGINX_WORKER_PROCESSES=1" \
    -e "KONG_DECLARATIVE_CONFIG=/kong/config/demo.yml" \
    -e "KONG_NGINX_WASM_SHM_KONG_WASM_RATE_LIMITING_COUNTERS=12m" \
    -e "KONG_PROXY_ACCESS_LOG=/dev/stdout" \
    -e "KONG_ADMIN_ACCESS_LOG=/dev/stdout" \
    -e "KONG_PROXY_ERROR_LOG=/dev/stderr" \
    -e "KONG_ADMIN_ERROR_LOG=/dev/stderr" \
    -e "KONG_ADMIN_LISTEN=0.0.0.0:8001" \
    -e "KONG_ADMIN_GUI_URL=http://localhost:8002" \
    -e "KONG_WASM_MODULES=$wasm_modules" \
    -e KONG_LICENSE_DATA \
    -p 8000:8000 \
    -p 8443:8443 \
    -p 8001:8001 \
    -p 8444:8444 \
    -p 8002:8002 \
    -p 8445:8445 \
    -p 8003:8003 \
    -p 8004:8004 \
     "$DEMO_KONG_IMAGE"

cat config/demo.yml

sleep 10

http :8000/echo

#docker stop $DEMO_KONG_CONTAINER
#docker rm $DEMO_KONG_CONTAINER

