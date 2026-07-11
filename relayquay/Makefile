ENV_FILE ?= /var/lib/relayquay/relayquay.env

.PHONY: init doctor up down stop status logs config

init:
	./bin/relayquay init

doctor:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay doctor

up:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay up

down:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay down

stop:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay stop

status:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay status

logs:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay logs

config:
	RELAYQUAY_ENV_FILE=$(ENV_FILE) ./bin/relayquay config
