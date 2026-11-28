import unittest

from openapi_parameters import validate_operation_parameters


class OpenApiParameterValidationTests(unittest.TestCase):
    def test_omitted_operation_parameters_are_allowed(self):
        validate_operation_parameters("/health/ready", "get", {}, {})

    def test_path_item_parameters_are_inherited_by_operation(self):
        path_item = {
            "parameters": [
                {"name": "item_id", "in": "path", "required": True, "schema": {"type": "string"}}
            ]
        }
        validate_operation_parameters("/items/{item_id}", "get", path_item, {})

    def test_mutations_still_require_idempotency_key(self):
        with self.assertRaisesRegex(AssertionError, "Idempotency-Key"):
            validate_operation_parameters("/items", "post", {}, {})


if __name__ == "__main__":
    unittest.main()
