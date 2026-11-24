---
title: Cloud Cost Management
section: Cloud Cost
order: 10
app: /cost
description: Daily costs from your cloud bills, Custom Costs files and Kubernetes clusters, explored by provider, service, account, workload or tag, and written as metrics for dashboards and monitors.
---

# Cloud Cost Management

Cloud Cost Management puts what you pay next to what you run. Robine Dog
gathers daily costs from three sources:

| Source | What it reads | Set up in |
| --- | --- | --- |
| Cloud bills | The billing exports your cloud writes to a bucket (FOCUS, or the AWS CUR), read every 6 hours | [Settings](/cost/settings) |
| Custom Costs | FOCUS files you upload, CSV or JSON, for any provider (SaaS, hosting, licences) | [Settings](/cost/settings) or the API |
| Kubernetes | A cluster's monthly price, split across its pods by their CPU and memory usage | [Settings](/cost/settings) |

The [Explorer](/cost) groups and compares them, and every cost is also
written as the `all.cost` and `<provider>.cost.amortized` metrics.

## File format

Cloud bills and Custom Costs use the same reader. A file is CSV (with a
header row) or JSON (an array of objects, or `{"data": [...]}`),
gzipped or not. Each row needs a date and a cost; the other columns are
optional. The first column present wins:

| Field | Columns read (FOCUS first, then AWS CUR) |
| --- | --- |
| Day | `ChargePeriodStart`, `BillingPeriodStart`, `lineItem/UsageStartDate`, `UsageDate`, `Date` (the first 10 characters, `YYYY-MM-DD`) |
| Cost | `EffectiveCost`, `BilledCost`, `lineItem/UnblendedCost`, `Cost` |
| Provider | `ProviderName`, `PublisherName`; `AWS` for a CUR file; else the source's default |
| Service | `ServiceName`, `product/ProductName`, `ChargeDescription`; else `unknown` |
| Account | `SubAccountId`, `SubAccountName`, `lineItem/UsageAccountId`, `BillingAccountId` |
| Resource | `ResourceId`, `lineItem/ResourceId`, `ResourceName` |
| Currency | `BillingCurrency`, `lineItem/CurrencyCode`, `Currency`; else `USD` |
| Tags | `Tags` (a JSON object), and the CUR's `resourceTags/user:<key>` columns |

Rows are summed per day, provider, service, account, resource, tags and
currency. Tag keys are lowercased, and characters other than letters,
digits, `_`, `.`, `-` and `/` become `_`. Rows without a date or a cost
are skipped; a file with no usable row is rejected.

The provider is reduced to one of `aws` (names containing *aws* or
*amazon*), `gcp` (*google* or `gcp`), `azure` (*azure* or *microsoft*),
`kubernetes`, or `custom` for everything else.

```csv
ChargePeriodStart,ChargePeriodEnd,BilledCost,BillingCurrency,ProviderName,ServiceName,SubAccountId,ResourceId,Tags
2026-11-01T00:00:00Z,2026-11-02T00:00:00Z,12.40,EUR,OVHcloud,Object Storage,prod-account,backups,"{""team"":""platform""}"
2026-11-01T00:00:00Z,2026-11-02T00:00:00Z,3.10,EUR,OVHcloud,Public IP,prod-account,ip-51-91,"{""team"":""platform""}"
```

## Cloud bills

In [Settings](/cost/settings), **Cloud accounts > New account**:

| Source | Fields | Use it for |
| --- | --- | --- |
| S3 bucket | Bucket, prefix, region (default `us-east-1`), endpoint (not for AWS), access key ID, secret access key | AWS Data Exports (FOCUS 1.0, CSV) or the CUR; Google Cloud Storage with HMAC keys (endpoint `https://storage.googleapis.com`); OVH, Garage, MinIO or any S3 API |
| Azure Storage container | Container SAS URL (with read and list rights), prefix | Cost Management exports (FOCUS, CSV) |
| HTTP URL | URL, optional header name and value (such as `Authorization`) | One FOCUS file at a URL |

*Provider* (`AWS` by default) names the provider for files that have no
`ProviderName` column. **Save and import** stores the account and imports it at once.

How bills are read:

- Every 6 hours (the first pass 5 minutes after Robine Dog starts), or when you click **Import now**.
- Under the prefix, only `.csv`, `.json` and `.gz` files whose name does not contain `manifest` are read; Parquet files are ignored.
- A file is read again only when its **ETag** changes. A file that disappears from the bucket has its costs removed.
- In a **billing period folder** (a path segment like `20261101-20261201/`), only the files of the latest run count: the AWS CUR and Azure write each run in a new sub-folder, and older runs are skipped.
- For an HTTP URL, the ETag (or `Last-Modified`) comes from a `HEAD` request; a server that gives neither is read again once a day.

The accounts list shows each source, the number of files imported, the
last import and its error, if any. Secrets are never shown again: leave a
secret field as it is to keep it. Deleting an account removes its costs.

## Custom Costs

Upload a file in **Settings > Custom Costs > Upload a file**, or with the
API:

```sh
curl -X PUT http://localhost:8080/api/v2/cost/custom_costs \
  -H "DD-API-KEY: $ROBINE_DOG_API_KEY" \
  -H "DD-APPLICATION-KEY: $ROBINE_DOG_APP_KEY" \
  -H "Content-Type: text/csv" \
  -H "X-File-Name: ovh-2026-11.csv" \
  --data-binary @ovh-2026-11.csv
```

The body is the file itself (CSV or JSON), or a multipart form whose
first part is the file. Rows without a `ProviderName` (and without CUR
columns) are `custom`.

| Method and path | Does |
| --- | --- |
| `PUT /api/v2/cost/custom_costs` | Uploads a file: its costs are added |
| `GET /api/v2/cost/custom_costs` | Lists files, newest first: name, rows, total `billed_cost`, currency, providers, charge period, upload time |
| `GET /api/v2/cost/custom_costs/{id}` | One file |
| `DELETE /api/v2/cost/custom_costs/{id}` | Deletes a file and its costs |

> **Note:** each upload is a new file. Uploading the same month twice
> counts it twice: delete the old file first.

## Kubernetes cost allocation

Add a cluster in **Settings > Kubernetes clusters > New cluster**:

| Field | Meaning |
| --- | --- |
| Name | The cluster, as shown in costs (`kube_cluster_name`) |
| Metrics filter | Tags that select its nodes and pods, such as `kube_cluster_name:silo01` (`*` for all) |
| Monthly price | What the cluster costs you per month (nodes, control plane…) |
| Currency | `EUR` by default |
| CPU share % | The part of the price paid for CPU, the rest for memory; 50 by default |

The cluster needs the [Datadog Agent](agent.md) with its Kubernetes
metrics. For each day, Robine Dog:

1. Takes the day's price: monthly price × 12 / 365, prorated to the hours the cluster reported metrics (a cluster watched since noon costs half a day).
2. Averages each pod's usage, `kubernetes.cpu.usage.total` and `kubernetes.memory.usage`, by namespace, pod and workload (`kube_deployment`, `kube_stateful_set`, `kube_daemon_set`).
3. Averages the nodes' capacity, `kubernetes_state.node.cpu_allocatable` and `kubernetes_state.node.memory_allocatable`.
4. Gives each pod `price × (cpu_share × pod CPU / CPU capacity + (1 − cpu_share) × pod memory / memory capacity)`.
5. Puts what is left, the unused capacity, in an `__idle__` row.

Without capacity metrics, the pods share the whole price. Costs are
computed for the last 7 days when you save a cluster, then every hour for
yesterday and today. **Recompute 30 days** redoes the last 30 days, after
you change the price for example.

[Kubernetes](/cost/kubernetes) lists the cost per cluster, namespace and
workload over the range (30 days by default), with the number of pods and
their average CPU and memory, plus the totals: cost, idle, and efficiency
(the share that is not idle).

## The Explorer

[Explorer](/cost) shows daily costs over the time range (30 days by
default), stacked by group, and a table of each group's cost and share.

- **Group by**: provider, service, account, resource, workload (the Kubernetes workload, else the resource, else the service), or **Tag…** with a tag key such as `team`. The 12 most expensive groups are shown; the rest is `other`.
- Filter on one provider.
- **Total** and **Previous period**: the cost of the range, the cost of the period of the same length right before it, and the change in percent.

Costs are summed as they are: when the range mixes currencies, the page
says so and does not convert them.

## Cost metrics

Every daily cost is also written as two metrics, timestamped at midnight
UTC of its day:

| Metric | Holds |
| --- | --- |
| `all.cost` | Every cost |
| `aws.cost.amortized`, `gcp.cost.amortized`, `azure.cost.amortized`, `kubernetes.cost.amortized`, `custom.cost.amortized` | The costs of that provider |

They are tagged `providername`, `servicename`, `account`, `cost_source` and
every tag of the cost (Kubernetes costs carry `kube_cluster_name`,
`kube_namespace` and their workload tag). Graph them in
[dashboards](dashboards.md) or alert on them with [monitors](monitors.md):

```text
sum:all.cost{providername:aws} by {servicename}.rollup(sum, 86400)
```

## Differences from Datadog

- Bills are read from FOCUS or CUR files only (CSV or JSON); Parquet exports and direct cloud billing integrations are not read.
- `EffectiveCost` is preferred over `BilledCost`; there are no separate amortized, unblended or net cost metrics.
- No currency conversion, budgets, cost recommendations or anomaly detection on costs.
- Kubernetes allocation uses one monthly price per cluster, not per-node cloud prices; GPU, storage and network are not allocated.
- Cloud accounts and clusters are configured in the UI only; the public API covers Custom Costs.
