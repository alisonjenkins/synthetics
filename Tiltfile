# Tilt: live local development of the synthetics fleet on KIND.
#
# Prereqs (see the justfile): `just kind-up` and `just cnpg-install` once, then
# `tilt up`. Tilt builds the OCI images with Nix, pushes them to the local
# registry, and deploys the Helm chart, redeploying on source changes.

REGISTRY = 'localhost:5001'

def nix_image(attr, image_name, ref):
    # Build the image with Nix, load it into Docker, tag as $EXPECTED_REF, push.
    custom_build(
        ref,
        ' && '.join([
            'set -e',
            'OUT=$(nix build .#%s --no-link --print-out-paths)' % attr,
            'docker load < $OUT',
            'docker tag %s:latest $EXPECTED_REF' % image_name,
            'docker push $EXPECTED_REF',
        ]),
        deps=['crates', 'Cargo.toml', 'Cargo.lock', 'flake.nix', 'flake.lock'],
        skips_local_docker=False,
    )

nix_image('control-plane-image', 'synthetics-control-plane', REGISTRY + '/synthetics-control-plane')
nix_image('worker-image', 'synthetics-worker', REGISTRY + '/synthetics-worker')

# Render + apply the Helm chart.
k8s_yaml(helm('deploy/helm/synthetics', name='synthetics'))

# Control plane: forward the API for local access.
k8s_resource(
    workload='synthetics-synthetics-control-plane',
    port_forwards=['8080:8080'],
    labels=['control-plane'],
)
k8s_resource(
    workload='synthetics-synthetics-worker',
    labels=['workers'],
)
