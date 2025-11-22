up:
	docker compose up -d

down:
	docker compose down

logs:
	docker compose logs -f

bootstrap:
	chmod +x bootstrap.sh
	./bootstrap.sh

reset:
	docker compose down -v
	docker compose up -d
	sleep 5
	./bootstrap.sh
