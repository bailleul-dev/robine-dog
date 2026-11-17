# Deployment

Production runs in silo01's k3s at https://robinedog.bailleul.dev, through
silo01's `robine_dog` role:

```sh
cd ../silo01
make robine-dog                                  # check, build, push, deploy, wait for /health
make robine-dog-deploy ROBINE_DOG_VERSION=<tag>  # roll out (or back) a pushed image
```

The Robine ID clients `robine-dog` and `robine-dog-dev` are declared in silo01
(`robine_id_applications`); their secrets, the session secret and the logs
intake key (`robine_dog_api_key`) live in silo01's `all.sops.yml`. The role
also runs GreptimeDB and PostgreSQL in the `robine-dog` namespace; see
silo01's `roles/robine_dog/README.md`.

The cluster's applications are connected through the Datadog Agent silo01
deploys next to it (`make datadog-agent`, `roles/datadog_agent`): container
logs namespace by namespace, cluster and host metrics, Traefik's access
logs, traces and metrics.
