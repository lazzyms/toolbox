import type { JobOutcome, Progress } from "../contracts";

export const ResultList = ({ results, progress, loading }: { results: JobOutcome[]; progress: Progress; loading: boolean }) => {
    if (loading) {
        return <p className="text-sm text-slate-500">Processing {progress.completed} of {progress.total} files...</p>;
    }
    if (results.length === 0) return null;

    const failures = results.filter((result) => result.failure !== null).length;
    return (
        <div className="space-y-2" aria-live="polite">
            <p className="text-sm font-medium text-slate-600">
                {failures === 0 ? `${results.length} files completed` : `${failures} of ${results.length} files failed`}
            </p>
            {results.map((result) => (
                <div key={result.inputPath} className="flex items-center justify-between rounded-lg border bg-white p-3">
                    <span className="max-w-xs truncate text-sm">{result.inputPath.split(/[\\/]/).pop()}</span>
                    <span className={`rounded px-2 py-1 text-xs font-medium ${result.failure ? "bg-red-100 text-red-600" : "bg-green-100 text-green-600"}`}>
                        {result.failure?.message ?? result.detail}
                    </span>
                </div>
            ))}
        </div>
    );
};
