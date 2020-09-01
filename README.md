# k8s-ares

ARES: Automatic REcord System.

A Kubernetes-native system to automatically create and manage DNS records
meant to run in parallel with External DNS.

Configuration is managed through the ares-secret Secret, typically in the
default namespace. This may change in the future to default to the
namespace that ARES is deployed in.

### Configuration

A configuration file should look like this:

```yaml
- selector:
  - syntixi.io
  provider: cloudflare
  providerOptions:
    apiToken: ***
```

The corresponding Secret can look like:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: ares-secret
stringData:
- selector:
  - syntixi.io
  provider: cloudflare
  providerOptions:
    apiToken: ***
```

If you want to control multiple domain zones across multiple different
providers, you can add another element into the default array and
configure another provider there. You can configure multiple domain zones
through a single provider.

### Custom Resource Definitions

ARES watches over the syntixi.io/v1alpha1/Record CustomResourceDefinition
to know which domain names to add, remove, or modify. An example resource
is below.

```yaml
apiVersion: syntixi.io/v1alpha1
kind: Record
metadata:
  name: example
spec:
  fqdn: example.syntixi.io
  ttl: 100
  type: CNAME
  value:
  - syntixi.io
```

For addresses that can change, such as Nodes that Pods may be running on,
it is recommended to instead use a valueFrom selector, such as the
PodSelector. The example below includes a Pod and a Record that points to
the Node the Pod is running on, with a Selector similar to that in the
Kubernetes
[documentation](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/).

This should not be used for inbound traffic (for that, you should use a
LoadBalancer Service or an Ingress record, with external-dns). This is,
however, useful for making SPF records point to an outbound mail record,
where the mail can be sent from one of many Nodes.

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: nginx-hello-world
  app: nginx
spec:
  containers:
  - name: nginx
    image: nginxdemos/hello
---
apiVersion: syntixi.io/v1alpha1
kind: Record
metadata:
  name: example-selector
spec:
  fqdn: selector.syntixi.io
  ttl: 1
  valueFrom:
    podSelector:
      matchLabels:
        app: nginx
```

When a syntixi.io/v1alpha1/Record resource is created, an additional record
is made for ARES to track ownership over the DNS record. So long as that
tracking record exists, when the Kubernetes resource is deleted, the
corresponding record and tracking record will be deleted.

License: MIT
