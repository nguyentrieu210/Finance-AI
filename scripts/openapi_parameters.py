import re


HTTP_METHODS = {"get", "put", "post", "delete", "options", "head", "patch", "trace"}
MUTATING_METHODS = {"post", "put", "patch", "delete"}


def validate_operation_parameters(path, method, path_item, operation):
    """Validate path placeholders against inherited and operation parameters.

    OpenAPI allows the parameters property to be omitted when an operation
    has none. Operation-level definitions override path-level definitions with
    the same (name, in) pair.
    """
    path_parameters = path_item.get("parameters", [])
    operation_parameters = operation.get("parameters", [])
    assert isinstance(path_parameters, list), f"{path}: path parameters must be a list"
    assert isinstance(operation_parameters, list), f"{path} {method}: parameters must be a list"

    effective = {}
    for parameter in path_parameters + operation_parameters:
        assert isinstance(parameter, dict), f"{path} {method}: invalid parameter"
        assert "name" in parameter and "in" in parameter, f"{path} {method}: parameter missing name or location"
        effective[(parameter["name"], parameter["in"])] = parameter

    placeholders = set(re.findall(r"\{(\w+)\}", path))
    declared_path_parameters = {
        parameter["name"]
        for parameter in effective.values()
        if parameter["in"] == "path"
    }
    assert placeholders == declared_path_parameters, (
        f"{path} {method}: path placeholders {sorted(placeholders)} do not match "
        f"declared parameters {sorted(declared_path_parameters)}"
    )
    if method in MUTATING_METHODS and operation.get("x-idempotent") is not True:
        assert any(
            parameter["name"] == "Idempotency-Key" and parameter.get("required")
            for parameter in effective.values()
        ), f"{path} {method}: required Idempotency-Key parameter missing"

 
def validate_path_item_parameters(path, path_item):
    """Check declared operations, ignoring Path Item metadata such as parameters."""
    assert isinstance(path_item, dict), f"{path}: path item must be a mapping"
    for method, operation in path_item.items():
        if method not in HTTP_METHODS:
            continue
        assert isinstance(operation, dict), f"{path} {method}: operation must be a mapping"
        validate_operation_parameters(path, method, path_item, operation)
