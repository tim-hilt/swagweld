# Swagweld

Swagweld discovers many OpenAPI spec files in a directory tree and welds them into a single API specification.

## Language

**Source Spec**:
One discovered OpenAPI file that contributes paths and components to the Bundle.
_Avoid_: input file, partial, fragment, root file

**Bundle**:
The single OpenAPI spec produced by merging all Source Specs.
_Avoid_: merged spec, dist, output spec

**Spec Name**:
The short name identifying a Source Spec: the filename prefix of `<name>.swagger.yaml`, otherwise its parent directory's name.
_Avoid_: namespace, prefix, service name

**Collision**:
Two Source Specs contributing the same name or path with different content.
_Avoid_: conflict, clash
