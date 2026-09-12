import React from 'react';
import { useTranslation } from 'react-i18next';

// The shared entryDisplay projection owns validation and historical compatibility.
export default function TranslationResult({ display, compact = false }) {
    const { t } = useTranslation();
    const symbols = display.pronunciations.map((item) => item.symbol).filter(Boolean);
    const sectionClass = compact ? 'mt-[6px] space-y-[4px]' : 'mt-[16px] space-y-[8px]';
    const mutedClass = compact ? 'text-[11px] text-default-500' : 'text-[12px] text-default-500';

    return (
        <div
            className={`flex flex-col min-w-0 max-w-full select-text whitespace-pre-wrap break-words [overflow-wrap:anywhere] ${
                compact ? 'text-[13px] gap-[2px]' : 'text-[14px] gap-[8px]'
            }`}
        >
            {symbols.length > 0 && <div className={mutedClass}>{symbols.join('  ')}</div>}
            {display.translation && <div>{display.translation}</div>}
            {display.explanations.map((item, index) => (
                <div key={index}>
                    {item.trait && (
                        <span className={`${compact ? 'text-[10px]' : 'text-[12px]'} text-default-400 mr-[6px]`}>
                            {item.trait}
                        </span>
                    )}
                    {item.explains.join(', ')}
                </div>
            ))}
            {display.associations.length > 0 && <div className={mutedClass}>{display.associations.join(', ')}</div>}
            {display.examples.length > 0 && (
                <section className={sectionClass}>
                    <h3 className={`${mutedClass} font-medium`}>{t('translate.examples')}</h3>
                    {display.examples.map((example, index) => (
                        <div key={index}>
                            <p>{example.text}</p>
                            <p className='text-default-500'>{example.translation}</p>
                        </div>
                    ))}
                </section>
            )}
            {display.notes.length > 0 && (
                <section className={sectionClass}>
                    <h3 className={`${mutedClass} font-medium`}>{t('translate.notes')}</h3>
                    {display.notes.map((note, index) => (
                        <p key={index}>{note}</p>
                    ))}
                </section>
            )}
            {display.syntax_breakdown && (
                <div className={sectionClass}>
                    {display.syntax_breakdown.main_clause && <p>{display.syntax_breakdown.main_clause}</p>}
                    {display.syntax_breakdown.clauses_and_modifiers && (
                        <p className='text-default-500'>{display.syntax_breakdown.clauses_and_modifiers}</p>
                    )}
                </div>
            )}
            {display.nuance_note && <p className={sectionClass}>{display.nuance_note}</p>}
            {display.key_vocabulary.length > 0 && (
                <div className={`${compact ? 'mt-[6px]' : 'mt-[16px]'} flex flex-wrap gap-[8px]`}>
                    {display.key_vocabulary.map((item, index) => (
                        <span
                            key={index}
                            className='max-w-full rounded-full border-1 border-default-200 px-[8px] py-[2px] text-[12px]'
                        >
                            {item.word} {item.meaning_in_context}
                        </span>
                    ))}
                </div>
            )}
        </div>
    );
}
