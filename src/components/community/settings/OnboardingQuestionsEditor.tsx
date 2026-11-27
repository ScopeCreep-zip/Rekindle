import { type Component, For } from "solid-js";
import type { Channel, Role } from "../../../stores/community.store";
import type { GuideStep, OnboardingConfigEdit, OnboardingOptionEdit, OnboardingQuestionEdit } from "../../../ipc/commands/dto";

interface OnboardingQuestionsEditorProps {
  flow: OnboardingConfigEdit;
  setFlow: (update: (f: OnboardingConfigEdit) => OnboardingConfigEdit) => void;
  channels: Channel[];
  roles: Role[];
}

/**
 * The onboarding questions, their options and the guide steps (plan E7.15
 * S2). This edits the fields only: the counts, lengths and ids are checked
 * by the Host's `OnboardingConfigSet` on Save (`validate::bounds`), and its
 * refusal names the field. A new question or option is sent with no id: the
 * Host mints it (plan E7.15 S3).
 */
const OnboardingQuestionsEditor: Component<OnboardingQuestionsEditorProps> = (props) => {
  const setQ = (qi: number, patch: Partial<OnboardingQuestionEdit>): void =>
    props.setFlow((f) => ({ ...f, questions: f.questions.map((q, j) => (j === qi ? { ...q, ...patch } : q)) }));
  const setO = (qi: number, oi: number, patch: Partial<OnboardingOptionEdit>): void =>
    setQ(qi, { options: props.flow.questions[qi].options.map((o, j) => (j === oi ? { ...o, ...patch } : o)) });
  const setStep = (si: number, patch: Partial<GuideStep>): void =>
    props.setFlow((f) => ({ ...f, guideSteps: f.guideSteps.map((s, j) => (j === si ? { ...s, ...patch } : s)) }));
  const toggle = (list: string[], id: string, on: boolean): string[] =>
    on ? [...list, id] : list.filter((x) => x !== id);

  return (
    <>
    <fieldset class="welcome-defaults">
      <legend class="form-field-label">Questions</legend>
      <For each={props.flow.questions}>
        {(q, qi) => (
          <div class="welcome-channel-row">
            <label class="welcome-field">
              <span class="form-field-label">Question</span>
              <input class="form-input" value={q.title} onInput={(e) => setQ(qi(), { title: e.currentTarget.value })} />
            </label>
            <label class="automod-enabled-toggle">
              <input type="checkbox" checked={q.required} onChange={(e) => setQ(qi(), { required: e.currentTarget.checked })} />
              <span>Required</span>
            </label>
            <label class="automod-enabled-toggle">
              <input type="checkbox" checked={q.singleSelect} onChange={(e) => setQ(qi(), { singleSelect: e.currentTarget.checked })} />
              <span>One answer only</span>
            </label>
            <For each={q.options}>
              {(o, oi) => (
                <div class="welcome-channel-row">
                  <label class="welcome-field">
                    <span class="form-field-label">Option</span>
                    <input class="form-input" value={o.title} onInput={(e) => setO(qi(), oi(), { title: e.currentTarget.value })} />
                  </label>
                  <fieldset class="welcome-defaults">
                    <legend class="form-field-label">Roles it gives</legend>
                    <For each={props.roles}>
                      {(r) => (
                        <label class="automod-enabled-toggle">
                          <input
                            type="checkbox"
                            checked={o.rolesToAssign.includes(r.id)}
                            onChange={(e) => setO(qi(), oi(), { rolesToAssign: toggle(o.rolesToAssign, r.id, e.currentTarget.checked) })}
                          />
                          <span>{r.name}</span>
                        </label>
                      )}
                    </For>
                  </fieldset>
                  <fieldset class="welcome-defaults">
                    <legend class="form-field-label">Channels it shows</legend>
                    <For each={props.channels}>
                      {(c) => (
                        <label class="automod-enabled-toggle">
                          <input
                            type="checkbox"
                            checked={o.channelsToShow.includes(c.id)}
                            onChange={(e) => setO(qi(), oi(), { channelsToShow: toggle(o.channelsToShow, c.id, e.currentTarget.checked) })}
                          />
                          <span>#{c.name}</span>
                        </label>
                      )}
                    </For>
                  </fieldset>
                  <button type="button" class="form-btn-secondary" onClick={() => setQ(qi(), { options: q.options.filter((_, j) => j !== oi()) })}>
                    Remove option
                  </button>
                </div>
              )}
            </For>
            <div class="settings-list-actions">
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => setQ(qi(), {
                  options: [...q.options, {
                    optionId: null, title: "", description: null, rolesToAssign: [], channelsToShow: [],
                  }],
                })}
              >
                Add option
              </button>
              <button
                type="button"
                class="form-btn-secondary"
                onClick={() => props.setFlow((f) => ({ ...f, questions: f.questions.filter((_, j) => j !== qi()) }))}
              >
                Remove question
              </button>
            </div>
          </div>
        )}
      </For>
      <button
        type="button"
        class="form-btn-secondary"
        onClick={() => props.setFlow((f) => ({
          ...f,
          questions: [...f.questions, {
            questionId: null, title: "", description: null, required: false, singleSelect: true, options: [],
          }],
        }))}
      >
        Add question
      </button>
    </fieldset>
    <fieldset class="welcome-defaults">
      <legend class="form-field-label">Guide steps</legend>
      <For each={props.flow.guideSteps}>
        {(s, si) => (
          <div class="welcome-channel-row">
            <label class="welcome-field">
              <span class="form-field-label">Step</span>
              <input class="form-input" value={s.title} onInput={(e) => setStep(si(), { title: e.currentTarget.value })} />
            </label>
            <label class="welcome-field">
              <span class="form-field-label">What to do</span>
              <input class="form-input" value={s.description} onInput={(e) => setStep(si(), { description: e.currentTarget.value })} />
            </label>
            <label class="welcome-field">
              <span class="form-field-label">Channel</span>
              <select
                class="form-select"
                value={s.channelId ?? ""}
                onChange={(e) => setStep(si(), { channelId: e.currentTarget.value === "" ? null : e.currentTarget.value })}
              >
                <option value="">None</option>
                <For each={props.channels}>{(c) => <option value={c.id}>#{c.name}</option>}</For>
              </select>
            </label>
            <button
              type="button"
              class="form-btn-secondary"
              onClick={() => props.setFlow((f) => ({ ...f, guideSteps: f.guideSteps.filter((_, j) => j !== si()) }))}
            >
              Remove step
            </button>
          </div>
        )}
      </For>
      <button
          type="button"
          class="form-btn-secondary"
          onClick={() => props.setFlow((f) => ({
            ...f,
            guideSteps: [...f.guideSteps, { title: "", description: "", channelId: null, emoji: null }],
          }))}
        >
          Add guide step
        </button>
    </fieldset>
    </>
  );
};

export default OnboardingQuestionsEditor;
