import type { OperationObject } from "fumadocs-openapi";

/** The default operation UI shows response schemas but not response headers. */
export function ResponseHeaders({ operation }: { operation: OperationObject }) {
  const responses = Object.entries(operation.responses ?? {}).filter(
    ([, response]) =>
      response &&
      "headers" in response &&
      Object.keys(response.headers ?? {}).length > 0,
  );
  if (!responses.length) return null;
  return (
    <section aria-labelledby="response-headers">
      <h2 id="response-headers">Response headers</h2>
      {responses.map(([status, response]) => (
        <div key={status}>
          <h3 id={`response-headers-${status}`}>{status}</h3>
          <table>
            <thead>
              <tr>
                <th>Header</th>
                <th>Description</th>
                <th>Schema</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(
                "headers" in response ? (response.headers ?? {}) : {},
              ).map(([name, value]) => {
                const header = value as {
                  description?: string;
                  schema?: unknown;
                };
                return (
                  <tr key={name}>
                    <td>
                      <code>{name}</code>
                    </td>
                    <td>{header.description}</td>
                    <td>
                      <code>{JSON.stringify(header.schema)}</code>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ))}
    </section>
  );
}
