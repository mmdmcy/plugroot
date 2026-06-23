ENV_FILE ?= /var/lib/plugroot/relayquay/relayquay.env

.PHONY: init doctor up down logs config key

init:
	./bin/relayquay init

doctor:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay doctor

up:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay up

down:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay down

logs:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay logs

config:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay config

key:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay key
