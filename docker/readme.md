# Usage

## Reth

- generate a JWT token:  `./docker/reth/generate-jwt.sh`
- Build reth image locally: `docker build -t reth:local -f docker/reth/Dockerfile .`
- Start reth node: `docker compose -f docker/docker-compose.yml -f docker/lighthouse.yml up -d`
- View reth log: `docker compose -f docker/docker-compose.yml -f docker/lighthouse.yml logs -f reth`

## Ress
