import { PROVIDERS, type Provider } from "../providers";

interface ProviderPickerProps {
  onPick: (provider: Provider) => void;
}

/**
 * Unavailable providers are listed rather than hidden, and stay clickable.
 *
 * Hiding them would suggest BazMail does not know about them; greying them out
 * silently would leave you guessing why. Picking one explains exactly what it is
 * waiting on, which is more useful than a disabled control that does nothing.
 */
export function ProviderPicker({ onPick }: ProviderPickerProps) {
  return (
    <div className="providers">
      {PROVIDERS.map((provider) => (
        <button
          key={provider.id}
          className={`provider ${provider.available ? "ready" : ""}`}
          onClick={() => onPick(provider)}
        >
          <span className="provider-name">{provider.name}</span>
          <span className="provider-method">{provider.method}</span>
          {provider.available ? (
            <span className="provider-badge ready">Ready</span>
          ) : (
            <span className="provider-badge">Soon</span>
          )}
        </button>
      ))}
    </div>
  );
}
