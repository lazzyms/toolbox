import packageInfo from "../../package.json";
import qrCode from "../assets/BuyMeACoffeeQR.png";

interface SettingsPanelProps {
    onClose: () => void;
}

export const SettingsPanel = ({ onClose }: SettingsPanelProps) => (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-slate-900/20 p-8" role="presentation">
        <section
            role="dialog"
            aria-modal="true"
            aria-labelledby="settings-title"
            className="max-h-full w-full max-w-lg overflow-y-auto rounded-2xl border border-slate-200 bg-white p-6 shadow-xl"
        >
            <header className="flex items-center justify-between border-b border-slate-200 pb-4">
                <h2 id="settings-title" className="text-xl font-semibold text-slate-900">Settings</h2>
                <button
                    type="button"
                    aria-label="Close settings"
                    onClick={onClose}
                    className="rounded-lg px-3 py-1.5 text-sm text-slate-500 transition-colors hover:bg-slate-100 hover:text-slate-900"
                >
                    Close
                </button>
            </header>

            <div className="space-y-6 pt-6">
                <section aria-labelledby="app-info-title">
                    <h3 id="app-info-title" className="text-sm font-semibold uppercase tracking-wide text-slate-500">App info</h3>
                    <div className="mt-3 rounded-xl border border-slate-200 bg-slate-50 p-4">
                        <p className="font-medium text-slate-900">Toolbox</p>
                        <p className="mt-1 text-sm text-slate-500">Version {packageInfo.version}</p>
                        <p className="mt-3 text-sm leading-6 text-slate-600">Private, local-first utilities for everyday PDF and image work.</p>
                    </div>
                </section>

                <section aria-labelledby="support-title">
                    <h3 id="support-title" className="text-sm font-semibold uppercase tracking-wide text-slate-500">Support Toolbox</h3>
                    <div className="mt-3 flex items-center gap-4 rounded-xl border border-slate-200 p-4">
                        <img
                            src={qrCode}
                            width="112"
                            height="112"
                            alt="Buy Me a Coffee donation QR code"
                            className="size-28 shrink-0 rounded-lg object-contain"
                        />
                        <div>
                            <p className="font-medium text-slate-900">Enjoying Toolbox?</p>
                            <p className="mt-1 text-sm leading-6 text-slate-500">If you’d like to support development, scan the QR code to support.</p>
                        </div>
                    </div>
                </section>
            </div>
        </section>
    </div>
);
