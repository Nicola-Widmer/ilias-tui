#!/bin/sh
# Waits for the local test ILIAS to finish its setup and fills it with test content.
set -e
cd "$(dirname "$0")"

echo "Waiting for ILIAS (the first start takes several minutes)..."
tries=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' -m 5 http://localhost:8080/)" = 200 ]; do
    tries=$((tries + 1))
    if [ "$tries" -gt 180 ]; then
        echo "ILIAS did not come up. See: docker compose -f dev/docker-compose.yml logs ilias" >&2
        exit 1
    fi
    sleep 5
done

# SOAP on, and no forced password change for the test user.
docker compose exec -T mysql mariadb -uroot -pilias-dev ilias -e "
    UPDATE settings SET value = '1' WHERE keyword = 'soap_user_administration';
    UPDATE settings SET value = 'http://localhost/soap/server.php?wsdl' WHERE keyword = 'soap_wsdl_path';
    DELETE FROM settings WHERE keyword = 'ps_password_change_on_first_login_enabled';
    INSERT INTO settings (module, keyword, value)
        VALUES ('common', 'ps_password_change_on_first_login_enabled', '0');"

docker compose exec -T -u www-data ilias php < seed.php 2>&1 | grep -v -E 'default_[a-z_]+\.(INFO|WARNING)' | tail -20

docker compose exec -T mysql mariadb -uroot -pilias-dev ilias -e "
    UPDATE usr_data SET last_password_change = UNIX_TIMESTAMP(), passwd_policy_reset = 0
        WHERE login = 'student';"
