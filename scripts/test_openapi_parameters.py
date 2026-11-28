import unittest

from openapi_parameters import validate_operation_parameters, validate_path_item_parameters


class OpenApiParameterValidationTests(unittest.TestCase):
    def test_omitted_operation_parameters_are_allowed(self):
        validate_operation_parameters("/health/ready", "get", {}, {})

    def test_path_item_parameters_are_inherited_by_operation(self):
        path_item = {
            "parameters": [
                {"name": "item_id", "in": "path", "required": True, "schema": {"type": "string"}}
            ]
        }
        path_item["get"] = {}
        validate_path_item_parameters("/items/{item_id}", path_item)

    def test_mutations_still_require_idempotency_key(self):
        with self.assertRaisesRegex(AssertionError, "Idempotency-Key"):
            validate_operation_parameters("/items", "post", {}, {})

    def test_query_parameter_cannot_satisfy_idempotency_header(self):
        with self.assertRaisesRegex(AssertionError, "Idempotency-Key"):
            validate_operation_parameters(
                "/items", "post", {},
                {"parameters": [{"name": "Idempotency-Key", "in": "query", "required": True}]},
            )

    def test_required_header_is_accepted(self):
        validate_operation_parameters(
            "/items", "post", {},
            {"parameters": [{"name": "Idempotency-Key", "in": "header", "required": True}]},
        )

    def test_explicitly_idempotent_mutation_can_omit_key(self):
        validate_operation_parameters(
            "/invitations/claim",
            "post",
            {},
            {"operationId": "claimStaffInvitation", "x-idempotent": True},
        )


if __name__ == "__main__":
    unittest.main()
